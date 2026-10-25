//! The reconnecting read loop: connects to gpsd, hands out reports and NMEA sentences, and connects
//! again when the connection fails or goes stale.

use std::{fmt, io, net::SocketAddr, thread, time::Duration};

use super::{
    connection::{ConnectError, Connection, Line, ReadError},
    protocol::{Report, Version, Watch},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Config {
    pub(crate) connect_timeout: Duration,
    /// Also bounds how long a stop request can go unnoticed while nothing arrives from gpsd.
    pub(crate) read_timeout: Duration,
    /// A longer gap between two returns from a read means that the thread was not reading: the
    /// process was frozen, the host was suspended, or the consumer was slow. Reports may have
    /// queued in the socket meanwhile, and must not be passed on as current. Normal gaps are
    /// about one read timeout or less.
    pub(crate) stale_gap: Duration,
    /// Pause before connecting again after a failure.
    pub(crate) reconnect_pause: Duration,
    /// Pauses are slept in slices this long, checking for a stop request between slices.
    pub(crate) pause_slice: Duration,
    /// Pause when the address closure gives no address.
    pub(crate) bad_address_pause: Duration,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            connect_timeout: Duration::from_secs(3),
            read_timeout: Duration::from_secs(1),
            stale_gap: Duration::from_secs(3),
            reconnect_pause: Duration::from_secs(2),
            pause_slice: Duration::from_millis(200),
            bad_address_pause: Duration::from_secs(5),
        }
    }
}

#[derive(Debug)]
pub(crate) enum Event {
    /// A connection is up and gpsd has sent its VERSION banner.
    Connected {
        addr: SocketAddr,
        version: Version,
    },
    Report(Report),
    /// An NMEA sentence, line ending included.
    Nmea(String),
    /// The address closure gave no address. The feed pauses and asks again.
    NoAddress,
    /// A connection attempt failed. The feed pauses and tries again.
    ConnectFailed {
        addr: SocketAddr,
        error: ConnectError,
    },
    /// The connection was dropped. The feed connects again, at once after a stale gap and after a
    /// pause otherwise.
    Disconnected {
        addr: SocketAddr,
        reason: Disconnect,
    },
}

#[derive(Debug)]
pub(crate) enum Disconnect {
    /// gpsd closed the connection.
    Closed,
    /// Too long between two reads; see [`Config::stale_gap`]. Closing the connection discards
    /// everything queued for it, and gpsd answers a new WATCH with no stored reports, so all
    /// that follows is new.
    Stale {
        gap: Duration,
    },
    Io(io::Error),
    LineTooLong,
    /// The first line was not a VERSION banner, so this is not gpsd.
    NoVersion,
    UnsupportedProtocol(Version),
    /// gpsd's echo of the watch settings does not have the watch enabled with JSON reports.
    WatchRefused(Watch),
}

impl fmt::Display for Disconnect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Disconnect::Closed => write!(f, "{}", ReadError::Closed),
            Disconnect::Stale { gap } => write!(
                f,
                "no read for {} ms; reconnecting to discard queued reports",
                gap.as_millis()
            ),
            Disconnect::Io(e) => write!(f, "read error: {e}"),
            Disconnect::LineTooLong => write!(f, "{}", ReadError::LineTooLong),
            Disconnect::NoVersion => write!(f, "first line was not a VERSION banner"),
            Disconnect::UnsupportedProtocol(v) => {
                write!(f, "{}", ReadError::UnsupportedProtocol(v.clone()))
            }
            Disconnect::WatchRefused(w) => write!(
                f,
                "gpsd did not enable the watch (enable {}, json {})",
                w.enable, w.json
            ),
        }
    }
}

/// A blocking iterator of events from gpsd. It ends, and closes any connection, when the stop
/// closure returns true. Closing the connection is how a client stops watching; gpsd treats it the
/// same as a WATCH request with `enable` false.
pub(crate) struct Feed {
    /// Asked before every connection attempt, so that the address can change between attempts.
    address: Box<dyn FnMut() -> Option<SocketAddr> + Send>,
    watch: Watch,
    config: Config,
    /// Checked after every read and during every pause.
    stop: Box<dyn Fn() -> bool + Send>,
    /// A monotonic clock for the stale-gap check. It should keep counting while the system is
    /// suspended, where the platform has such a clock.
    clock: Box<dyn Fn() -> Duration + Send>,
    state: State,
}

enum State {
    /// Connect at the next call.
    Connect,
    /// Pause this long, then connect.
    Pause(Duration),
    Connected(Open),
}

/// An open connection and what the feed remembers about it.
struct Open {
    conn: Connection,
    /// Clock reading at the last return from a read, or at connection.
    last_read: Duration,
    /// Whether the VERSION banner has arrived.
    banner_seen: bool,
}

/// What one pass of the loop decided.
enum Step {
    Continue,
    Emit(Event),
    Drop {
        addr: SocketAddr,
        reason: Disconnect,
        pause: bool,
    },
}

impl Open {
    fn read(&mut self, clock: &dyn Fn() -> Duration, stale_gap: Duration) -> Step {
        let result = self.conn.read();
        // The clock is read after the read returns, so that a long read counts as well as a long gap
        // between reads: a process frozen while blocked in the read gets the first queued report from
        // this very read.
        let now = clock();
        let addr = self.conn.peer();
        let gap = now.saturating_sub(self.last_read);
        self.last_read = now;
        if gap > stale_gap {
            return Step::Drop {
                addr,
                reason: Disconnect::Stale { gap },
                pause: false,
            };
        }
        let fail = |reason| Step::Drop {
            addr,
            reason,
            pause: true,
        };
        match result {
            Ok(Line::Json(Report::Version(version))) if !self.banner_seen => {
                self.banner_seen = true;
                Step::Emit(Event::Connected { addr, version })
            }
            Ok(_) if !self.banner_seen => fail(Disconnect::NoVersion),
            Ok(Line::Json(Report::Watch(echo))) if !(echo.enable && echo.json) => {
                fail(Disconnect::WatchRefused(echo))
            }
            Ok(Line::Json(report)) => Step::Emit(Event::Report(report)),
            Ok(Line::Nmea(sentence)) => Step::Emit(Event::Nmea(sentence)),
            Err(ReadError::Timeout) => Step::Continue,
            Err(ReadError::Parse { line, error }) => {
                log::warn!("gpsd sent a line that is not a report ({error}): {line}");
                Step::Continue
            }
            Err(ReadError::Closed) => fail(Disconnect::Closed),
            Err(ReadError::Io(e)) => fail(Disconnect::Io(e)),
            Err(ReadError::LineTooLong) => fail(Disconnect::LineTooLong),
            Err(ReadError::UnsupportedProtocol(v)) => fail(Disconnect::UnsupportedProtocol(v)),
        }
    }
}

impl Feed {
    pub(crate) fn new(
        address: impl FnMut() -> Option<SocketAddr> + Send + 'static,
        watch: Watch,
        config: Config,
        stop: impl Fn() -> bool + Send + 'static,
        clock: impl Fn() -> Duration + Send + 'static,
    ) -> Feed {
        Feed {
            address: Box::new(address),
            watch,
            config,
            stop: Box::new(stop),
            clock: Box::new(clock),
            state: State::Connect,
        }
    }

    /// Sleeps for `total`, a slice at a time. Returns false if a stop was requested meanwhile.
    fn pause(&self, total: Duration) -> bool {
        let slice = self.config.pause_slice.max(Duration::from_millis(1));
        let mut left = total;
        while !left.is_zero() {
            if (self.stop)() {
                return false;
            }
            let step = left.min(slice);
            thread::sleep(step);
            left -= step;
        }
        !(self.stop)()
    }

    fn connect(&mut self) -> Option<Event> {
        let Some(addr) = (self.address)() else {
            self.state = State::Pause(self.config.bad_address_pause);
            return Some(Event::NoAddress);
        };
        match Connection::connect(
            addr,
            &self.watch,
            self.config.connect_timeout,
            self.config.read_timeout,
        ) {
            Ok(conn) => {
                self.state = State::Connected(Open {
                    conn,
                    last_read: (self.clock)(),
                    banner_seen: false,
                });
                None
            }
            Err(error) => {
                self.state = State::Pause(self.config.reconnect_pause);
                Some(Event::ConnectFailed { addr, error })
            }
        }
    }
}

impl Iterator for Feed {
    type Item = Event;

    fn next(&mut self) -> Option<Event> {
        loop {
            if (self.stop)() {
                // Dropping the connection closes the socket, which ends the watch in gpsd.
                self.state = State::Connect;
                return None;
            }
            let step = match &mut self.state {
                State::Connect => {
                    if let Some(event) = self.connect() {
                        return Some(event);
                    }
                    continue;
                }
                State::Pause(total) => {
                    let total = *total;
                    self.state = State::Connect;
                    if !self.pause(total) {
                        return None;
                    }
                    continue;
                }
                State::Connected(open) => open.read(&*self.clock, self.config.stale_gap),
            };
            match step {
                Step::Continue => {}
                Step::Emit(event) => return Some(event),
                Step::Drop { addr, reason, pause } => {
                    self.state = if pause {
                        State::Pause(self.config.reconnect_pause)
                    } else {
                        State::Connect
                    };
                    return Some(Event::Disconnected { addr, reason });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, AtomicU64, Ordering},
            Arc,
        },
        time::Instant,
    };

    use super::{
        super::{
            protocol::samples::*,
            tests::{line, FakeGpsd, Step},
        },
        *,
    };

    /// Short timeouts, so that the tests run in well under a second each.
    fn fast() -> Config {
        Config {
            connect_timeout: Duration::from_secs(2),
            read_timeout: Duration::from_millis(100),
            stale_gap: Duration::from_secs(3),
            reconnect_pause: Duration::from_millis(100),
            pause_slice: Duration::from_millis(10),
            bad_address_pause: Duration::from_millis(100),
        }
    }

    struct Controls {
        clock: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
    }

    impl Controls {
        fn advance(&self, d: Duration) {
            self.clock.fetch_add(d.as_nanos() as u64, Ordering::SeqCst);
        }
    }

    /// A feed with a clock the test moves and a stop flag the test sets.
    fn feed(
        address: impl FnMut() -> Option<SocketAddr> + Send + 'static,
        config: Config,
    ) -> (Feed, Controls) {
        let clock = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (c, s) = (Arc::clone(&clock), Arc::clone(&stop));
        let feed = Feed::new(
            address,
            Watch::default(),
            config,
            move || s.load(Ordering::SeqCst),
            move || Duration::from_nanos(c.load(Ordering::SeqCst)),
        );
        (feed, Controls { clock, stop })
    }

    fn tpv(lat: f64) -> String {
        format!(r#"{{"class":"TPV","mode":2,"lat":{lat:.1},"lon":0.0}}"#)
    }

    fn handshake() -> Vec<Step> {
        vec![line(VERSION), Step::ExpectRequest("?WATCH=")]
    }

    /// The event must be a TPV with this latitude.
    fn assert_tpv(event: Option<Event>, lat: f64) {
        match event {
            Some(Event::Report(Report::Tpv(t))) => assert_eq!(t.lat, Some(lat)),
            other => panic!("expected a TPV with latitude {lat}, got {other:?}"),
        }
    }

    #[test]
    fn feed_can_move_to_a_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<Feed>();
    }

    #[test]
    fn stale_gap_reconnects_at_once_and_drops_queued_reports() {
        let mut first = handshake();
        first.extend([line(&tpv(1.0)), line(&tpv(2.0)), Step::WaitForClose]);
        let mut second = handshake();
        second.extend([line(&tpv(3.0)), Step::WaitForClose]);
        let fake = FakeGpsd::start(vec![first, second]);
        // A long pause, to show that a stale gap does not wait for it.
        let config = Config {
            reconnect_pause: Duration::from_secs(5),
            ..fast()
        };
        let (mut feed, controls) = feed(move || Some(fake.addr), config);

        assert!(matches!(feed.next(), Some(Event::Connected { .. })));
        assert_tpv(feed.next(), 1.0);
        // The second TPV is already in the socket when the thread stops reading for five seconds.
        controls.advance(Duration::from_secs(5));
        assert!(matches!(
            feed.next(),
            Some(Event::Disconnected { reason: Disconnect::Stale { gap }, .. })
                if gap == Duration::from_secs(5)
        ));
        let before = Instant::now();
        assert!(matches!(feed.next(), Some(Event::Connected { .. })));
        assert!(
            before.elapsed() < Duration::from_secs(1),
            "paused before reconnecting"
        );
        // The queued TPV must not be delivered.
        assert_tpv(feed.next(), 3.0);
    }

    #[test]
    fn a_gap_during_a_read_drops_the_report_that_read_returns() {
        // The process is frozen while blocked in a read, and the first queued report arrives in the
        // same read call as the thaw. The gap must be measured when that read returns, not when it
        // began.
        let mut first = handshake();
        first.extend([
            Step::Sleep(Duration::from_millis(400)),
            line(&tpv(2.0)),
            Step::WaitForClose,
        ]);
        let mut second = handshake();
        second.extend([line(&tpv(3.0)), Step::WaitForClose]);
        let fake = FakeGpsd::start(vec![first, second]);
        let addr = fake.addr;
        // A read timeout longer than the server's pause, so that one read call spans the clock jump.
        let config = Config {
            read_timeout: Duration::from_secs(2),
            ..fast()
        };
        let (mut feed, controls) = feed(move || Some(addr), config);

        assert!(matches!(feed.next(), Some(Event::Connected { .. })));
        let clock = Arc::clone(&controls.clock);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            clock.fetch_add(Duration::from_secs(5).as_nanos() as u64, Ordering::SeqCst);
        });
        assert!(matches!(
            feed.next(),
            Some(Event::Disconnected {
                reason: Disconnect::Stale { .. },
                ..
            })
        ));
        assert!(matches!(feed.next(), Some(Event::Connected { .. })));
        assert_tpv(feed.next(), 3.0);
    }

    #[test]
    fn stop_ends_the_feed_and_closes_the_connection() {
        let mut script = handshake();
        script.push(Step::WaitForClose);
        let fake = FakeGpsd::start(vec![script]);
        let addr = fake.addr;
        let (mut feed, controls) = feed(move || Some(addr), fast());

        assert!(matches!(feed.next(), Some(Event::Connected { .. })));
        controls.stop.store(true, Ordering::SeqCst);
        let before = Instant::now();
        assert!(feed.next().is_none());
        assert!(before.elapsed() < Duration::from_millis(500));
        assert!(fake.wait_closed(1, Duration::from_secs(2)), "socket not closed");
        assert!(feed.next().is_none());
    }

    #[test]
    fn reconnects_after_a_pause_when_gpsd_closes() {
        let mut first = handshake();
        first.push(Step::Close);
        let mut second = handshake();
        second.extend([line(&tpv(3.0)), Step::WaitForClose]);
        let fake = FakeGpsd::start(vec![first, second]);
        let addr = fake.addr;
        let config = fast();
        let pause = config.reconnect_pause;
        let (mut feed, _controls) = feed(move || Some(addr), config);

        assert!(matches!(feed.next(), Some(Event::Connected { .. })));
        assert!(matches!(
            feed.next(),
            Some(Event::Disconnected {
                reason: Disconnect::Closed,
                ..
            })
        ));
        let before = Instant::now();
        assert!(matches!(feed.next(), Some(Event::Connected { .. })));
        assert!(before.elapsed() >= pause, "reconnected without the pause");
        assert_tpv(feed.next(), 3.0);
    }

    #[test]
    fn address_problems_are_reported_and_retried() {
        let mut script = handshake();
        script.push(Step::WaitForClose);
        let fake = FakeGpsd::start(vec![script]);
        // A port nobody listens on: bind one and drop it.
        let dead = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let good = fake.addr;
        let mut attempt = 0;
        let address = move || {
            attempt += 1;
            match attempt {
                1 => None,
                2 => Some(dead),
                _ => Some(good),
            }
        };
        let (mut feed, _controls) = feed(address, fast());

        assert!(matches!(feed.next(), Some(Event::NoAddress)));
        assert!(matches!(
            feed.next(),
            Some(Event::ConnectFailed { addr, error: ConnectError::Io(_) }) if addr == dead
        ));
        assert!(matches!(feed.next(), Some(Event::Connected { addr, .. }) if addr == good));
    }

    #[test]
    fn a_server_that_does_not_behave_like_gpsd_is_dropped() {
        // No VERSION banner first.
        let first = vec![line(&tpv(1.0)), Step::WaitForClose];
        // A watch echo that says the watch is off.
        let mut second = handshake();
        second.extend([line(WATCH_REFUSED), Step::WaitForClose]);
        let mut third = handshake();
        third.push(Step::WaitForClose);
        let fake = FakeGpsd::start(vec![first, second, third]);
        let addr = fake.addr;
        let (mut feed, _controls) = feed(move || Some(addr), fast());

        assert!(matches!(
            feed.next(),
            Some(Event::Disconnected {
                reason: Disconnect::NoVersion,
                ..
            })
        ));
        assert!(matches!(feed.next(), Some(Event::Connected { .. })));
        assert!(matches!(
            feed.next(),
            Some(Event::Disconnected {
                reason: Disconnect::WatchRefused(_),
                ..
            })
        ));
        assert!(matches!(feed.next(), Some(Event::Connected { .. })));
    }
}
