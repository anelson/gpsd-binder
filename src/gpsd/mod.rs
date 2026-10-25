//! Client for gpsd's JSON protocol over TCP.
//!
//! [`protocol`] holds the report and request types, [`connection`] one connection to gpsd, and
//! [`feed`] the reconnecting loop that the driver runs on a thread. Nothing here knows about
//! Android.
//!
//! Behaviour relied on, verified in gpsd 3.27.5 and `gpsd_json(5)`: gpsd sends a VERSION banner on
//! connect; it answers `?WATCH` with the device list and the watch settings, never with stored
//! reports; closing the connection ends the watch; request lines are at most 80 characters; when a
//! client's socket is full, gpsd drops reports and then the client.

mod connection;
mod feed;
mod protocol;

pub(crate) use feed::{Config, Event, Feed};
#[cfg(test)]
pub(crate) use protocol::samples;
pub(crate) use protocol::{GnssId, Report, Satellite, Tpv, Watch};

/// The scripted stand-in for gpsd that the connection and feed tests use, and the test of the whole
/// module against a real gpsd.
#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
        path::Path,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc, Mutex,
        },
        thread,
        time::{Duration, Instant},
    };

    use testcontainers::{
        core::{IntoContainerPort, WaitFor},
        runners::{SyncBuilder, SyncRunner},
        Container, GenericBuildableImage, GenericImage,
    };

    use super::{
        connection::{Connection, ReadError},
        protocol::MIN_PROTO_MAJOR,
        *,
    };

    /// One step of a fake gpsd's script. Each accepted connection runs one script and then closes.
    pub(super) enum Step {
        /// Write this text as it is; [`line`] adds the newline.
        Send(String),
        /// Read request lines until one starts with this prefix.
        ExpectRequest(&'static str),
        Sleep(Duration),
        /// Keep the connection open until the client closes it.
        WaitForClose,
        Close,
    }

    pub(super) fn line(s: &str) -> Step {
        Step::Send(format!("{s}\n"))
    }

    pub(super) struct FakeGpsd {
        pub(super) addr: SocketAddr,
        requests: Arc<Mutex<Vec<String>>>,
        closed: Arc<AtomicUsize>,
    }

    impl FakeGpsd {
        /// Listens on a free loopback port and serves one script per connection, in order. Further
        /// connections are accepted and closed at once.
        pub(super) fn start(scripts: Vec<Vec<Step>>) -> FakeGpsd {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let closed = Arc::new(AtomicUsize::new(0));
            let (requests2, closed2) = (Arc::clone(&requests), Arc::clone(&closed));
            thread::spawn(move || {
                let mut scripts = scripts.into_iter();
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    if let Some(script) = scripts.next() {
                        serve(stream, script, &requests2);
                    }
                    closed2.fetch_add(1, Ordering::SeqCst);
                }
            });
            FakeGpsd {
                addr,
                requests,
                closed,
            }
        }

        /// Every request line received so far, over all connections.
        pub(super) fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }

        /// Waits until `n` connections have ended.
        pub(super) fn wait_closed(&self, n: usize, timeout: Duration) -> bool {
            let deadline = Instant::now() + timeout;
            while self.closed.load(Ordering::SeqCst) < n {
                if Instant::now() > deadline {
                    return false;
                }
                thread::sleep(Duration::from_millis(10));
            }
            true
        }
    }

    fn serve(stream: TcpStream, script: Vec<Step>, requests: &Mutex<Vec<String>>) {
        // A bound on every read, so that a failing test cannot leave this thread waiting forever.
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        for step in script {
            match step {
                Step::Send(s) => {
                    if writer.write_all(s.as_bytes()).is_err() {
                        return;
                    }
                }
                Step::ExpectRequest(prefix) => loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => return,
                        Ok(_) => {
                            requests.lock().unwrap().push(line.clone());
                            if line.starts_with(prefix) {
                                break;
                            }
                        }
                    }
                },
                Step::Sleep(d) => thread::sleep(d),
                Step::WaitForClose => {
                    let mut buf = [0u8; 256];
                    while let Ok(n) = reader.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                    }
                    return;
                }
                Step::Close => return,
            }
        }
    }

    const GPSD_PORT: u16 = 2947;
    /// Bound on the streaming part of the container test. The image build and the container start
    /// are bounded by Docker and testcontainers.
    const DEADLINE: Duration = Duration::from_secs(60);

    /// The whole module against a real gpsd, fed by gpsfake in a container. Needs a running Docker
    /// daemon; see testdata/README.md.
    #[test]
    fn real_gpsd_over_tcp() {
        let testdata = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/gpsd/testdata");
        let image = GenericBuildableImage::new("gpsd-binder-test-gpsd", "trixie")
            .with_dockerfile(testdata.join("Dockerfile"))
            .with_file(testdata.join("neo-m8n.log"), "neo-m8n.log")
            .build_image()
            .expect("building the gpsd image needs a running Docker daemon");
        let container = image
            .with_exposed_port(GPSD_PORT.tcp())
            // gpsfake writes this when gpsd is up and the replay starts.
            .with_wait_for(WaitFor::message_on_stderr("log cycle of"))
            .start()
            .expect("start the gpsd container");
        let addr = host_addr(&container);
        let deadline = Instant::now() + DEADLINE;

        // Everything the driver needs, through the feed.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let started = Instant::now();
        let mut feed = Feed::new(
            move || Some(addr),
            Watch::default(),
            Config::default(),
            move || stop_flag.load(Ordering::SeqCst),
            move || started.elapsed(),
        );
        let (mut version, mut devices, mut watch, mut fix, mut sky) = (None, None, None, None, None);
        while Instant::now() < deadline
            && (version.is_none() || devices.is_none() || watch.is_none() || fix.is_none() || sky.is_none())
        {
            match feed.next().expect("the feed ended") {
                Event::Connected { version: v, .. } => version = Some(v),
                Event::Report(Report::Devices(d)) => devices = Some(d),
                Event::Report(Report::Watch(w)) => watch = Some(w),
                // gpsd also reports fixes without a time stamp, from sentences that carry no date; the
                // driver needs one with.
                Event::Report(Report::Tpv(t))
                    if t.mode.has_fix() && t.lat.is_some() && t.lon.is_some() && t.time.is_some() =>
                {
                    fix = Some(t);
                }
                Event::Report(Report::Sky(s)) if s.satellites.as_ref().is_some_and(|v| !v.is_empty()) => {
                    sky = Some(s);
                }
                Event::Report(_) => {}
                other => panic!("unexpected event: {other:?}"),
            }
        }
        stop.store(true, Ordering::SeqCst);
        assert!(feed.next().is_none());

        let version = version.expect("no VERSION");
        assert!(version.proto_major >= MIN_PROTO_MAJOR, "{version:?}");
        let devices = devices.expect("no DEVICES");
        let path = devices.devices[0].path.clone().expect("device without a path");
        let watch = watch.expect("no WATCH reply");
        assert!(watch.enable && watch.json, "{watch:?}");
        let fix = fix.expect("no TPV with a fix, position and time");
        assert!(fix.alt_hae.is_some(), "{fix:?}");
        let sky = sky.expect("no SKY with satellites");
        let sats = sky.satellites.as_deref().unwrap();
        assert!(
            sats.iter()
                .any(|s| s.el.is_some() && s.az.is_some() && s.ss.is_some()),
            "{sats:?}"
        );
        assert!(sats.iter().any(|s| s.used), "{sats:?}");
        assert!(
            sats.iter()
                .any(|s| s.gnssid == Some(GnssId::Gps) && s.svid.is_some()),
            "{sats:?}"
        );

        // A watch on the one device by name: gpsd echoes the name and reports keep coming.
        let one_device = Watch {
            device: Some(path.clone()),
            ..Watch::default()
        };
        let mut conn = connect(addr, &one_device);
        let echo = read_until(&mut conn, deadline, |r| matches!(r, Report::Watch(_)));
        assert!(matches!(echo, Report::Watch(w) if w.device.as_deref() == Some(path.as_str())));
        read_until(&mut conn, deadline, |r| matches!(r, Report::Tpv(_)));

        // A watch on a device gpsd does not have: an ERROR report.
        let no_device = Watch {
            device: Some("/dev/pts/none".to_string()),
            ..Watch::default()
        };
        let mut conn = connect(addr, &no_device);
        let report = read_until(&mut conn, deadline, |r| matches!(r, Report::Error(_)));
        assert!(matches!(report, Report::Error(e) if e.message.contains("No such device")));
    }

    /// The published port, as an address this host can connect to.
    fn host_addr(container: &Container<GenericImage>) -> SocketAddr {
        let port = container.get_host_port_ipv4(GPSD_PORT.tcp()).unwrap();
        let host = container.get_host().unwrap().to_string();
        let ip: IpAddr = host.parse().unwrap_or_else(|_| {
            (host.as_str(), port)
                .to_socket_addrs()
                .unwrap()
                .map(|a| a.ip())
                .find(IpAddr::is_ipv4)
                .expect("an IPv4 address for the Docker host")
        });
        SocketAddr::new(ip, port)
    }

    fn connect(addr: SocketAddr, watch: &Watch) -> Connection {
        Connection::connect(addr, watch, Duration::from_secs(3), Duration::from_secs(1)).unwrap()
    }

    /// Reads until a report satisfies `wanted`, skipping the others.
    fn read_until(conn: &mut Connection, deadline: Instant, wanted: fn(&Report) -> bool) -> Report {
        while Instant::now() < deadline {
            match conn.read() {
                Ok(report) if wanted(&report) => return report,
                Ok(_) | Err(ReadError::Timeout) => {}
                Err(e) => panic!("read failed: {e}"),
            }
        }
        panic!("no matching report before the deadline");
    }
}
