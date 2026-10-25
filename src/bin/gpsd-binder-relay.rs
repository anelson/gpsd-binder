//! The relay program, `/vendor/bin/gpsd-binder-relay` once installed. The GNSS service starts it.
//! It copies bytes between the service and gpsd without parsing the protocol.

use std::{
    io,
    net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs},
    os::{fd::AsFd, unix::net::UnixStream},
    process::ExitCode,
    thread,
    time::Duration,
};

fn main() -> ExitCode {
    gpsd_binder::init_logging("gpsd-binder-relay");
    match Forwarder::from_stdin().and_then(Forwarder::run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::warn!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// The two connected sockets owned by one relay child.
struct Forwarder {
    service: UnixStream,
    gpsd: TcpStream,
}

impl Forwarder {
    /// Reads the address for this connection. Changed properties take effect on the next child.
    fn from_stdin() -> io::Result<Self> {
        let service = UnixStream::from(io::stdin().as_fd().try_clone_to_owned()?);
        // Numeric, not "localhost": only the first resolved address is tried, and that could be ::1.
        let host = Self::property("vendor.gpsd.host", "127.0.0.1");
        let port = Self::property("vendor.gpsd.port", "2947");
        let port = port.parse::<u16>().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("vendor.gpsd.port {port:?}: {e}"),
            )
        })?;
        let addr = (host.as_str(), port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no address for {host}")))?;
        Self::connect(service, addr)
    }

    /// Android treats unset and empty properties alike.
    fn property(name: &str, default: &str) -> String {
        let value: String = rsproperties::get_or(name, String::new());
        if value.is_empty() {
            default.to_owned()
        } else {
            value
        }
    }

    fn connect(service: UnixStream, addr: SocketAddr) -> io::Result<Self> {
        let gpsd = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
            .map_err(|e| io::Error::new(e.kind(), format!("connect {addr}: {e}")))?;
        log::info!("connected to gpsd at {addr}");
        Ok(Forwarder { service, gpsd })
    }

    /// Copies until either peer closes, then shuts down both sockets to wake the other copy.
    fn run(self) -> io::Result<()> {
        let Self { service, gpsd } = self;
        thread::scope(|scope| {
            let upstream = scope.spawn(|| {
                let copied = io::copy(&mut &service, &mut &gpsd);
                let _ = gpsd.shutdown(Shutdown::Both);
                let _ = service.shutdown(Shutdown::Both);
                copied
            });
            let downstream = io::copy(&mut &gpsd, &mut &service);
            let _ = service.shutdown(Shutdown::Both);
            let _ = gpsd.shutdown(Shutdown::Both);
            let upstream = upstream.join().expect("the copying thread does not panic");
            downstream.and(upstream).map(|_| ())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
    };

    use super::*;

    #[test]
    fn copies_bytes_and_exits_when_either_peer_closes() {
        for close_service in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let (mut service, child_socket) = UnixStream::pair().unwrap();
            let forwarder = Forwarder::connect(child_socket, listener.local_addr().unwrap()).unwrap();
            let (mut gpsd, _) = listener.accept().unwrap();
            let timeout = Duration::from_secs(2);
            service.set_read_timeout(Some(timeout)).unwrap();
            gpsd.set_read_timeout(Some(timeout)).unwrap();
            let (done, finished) = mpsc::channel();
            let child = thread::spawn(move || done.send(forwarder.run()).unwrap());

            const REQUEST: &[u8] = b"?WATCH={\"enable\":true};\n";
            const SENTENCE: &[u8] = b"$GNGGA,\r\n";
            service.write_all(REQUEST).unwrap();
            let mut request = [0; REQUEST.len()];
            gpsd.read_exact(&mut request).unwrap();
            assert_eq!(&request, REQUEST);
            gpsd.write_all(SENTENCE).unwrap();
            let mut sentence = [0; SENTENCE.len()];
            service.read_exact(&mut sentence).unwrap();
            assert_eq!(&sentence, SENTENCE);

            let mut byte = [0];
            if close_service {
                service.shutdown(Shutdown::Both).unwrap();
                assert_eq!(gpsd.read(&mut byte).unwrap(), 0);
            } else {
                gpsd.shutdown(Shutdown::Both).unwrap();
                assert_eq!(service.read(&mut byte).unwrap(), 0);
            }
            finished
                .recv_timeout(timeout)
                .expect("the relay did not exit")
                .unwrap();
            child.join().unwrap();
        }
    }
}
