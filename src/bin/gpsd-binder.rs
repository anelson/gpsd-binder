//! The GNSS service program, `android.hardware.gnss-service.gpsd-binder` once installed.
//!
//! Android's SELinux policy forbids HAL servers from using TCP sockets. Therefore this gets around
//! that by spawning the `gpsd-binder-relay` process which SELinux policy does allow to use TCP,
//! using UDS connected to that relay process to pass bytes back and forth between the TCP
//! connection to gpsd and the server.

use std::{
    io::{self, Read, Write},
    os::{fd::OwnedFd, unix::net::UnixStream},
    process::{Child, Command, ExitCode},
    time::Duration,
};

use gpsd_binder::Stream;

fn main() -> ExitCode {
    gpsd_binder::init_logging("gpsd-binder");
    match gpsd_binder::run_service(|| Ok(Box::new(RelayClient::start()?))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("binder: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The service's connection to the relay child process.
///
/// Dropping this connection kills and reaps the child process.
///
/// This implements the [`Stream`] trait to expose the relay process's UDS as a stream of bytes that
/// the service can use to communicate with gpsd.
struct RelayClient {
    stream: UnixStream,
    child: Child,
}

impl RelayClient {
    /// Starts the child relay process with a full-duplex Unix socket as stdin. A failed TCP
    /// connection in the child closes this stream, which the service can detect and then retry
    /// with a new child.
    fn start() -> io::Result<Self> {
        let (stream, child_socket) = UnixStream::pair()?;
        let child = Command::new("/vendor/bin/gpsd-binder-relay")
            .stdin(OwnedFd::from(child_socket))
            .spawn()?;
        Ok(RelayClient { stream, child })
    }
}

impl Read for RelayClient {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl Write for RelayClient {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl Stream for RelayClient {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.set_read_timeout(timeout)
    }
}

impl Drop for RelayClient {
    fn drop(&mut self) {
        // Closing the socket alone cannot interrupt a child that is still connecting to gpsd.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
