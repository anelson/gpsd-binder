//! One TCP connection to gpsd: the WATCH request on the way in, reports and NMEA sentences read one
//! line at a time on the way out.

use std::{
    fmt,
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use super::protocol::{Report, RequestTooLong, Version, Watch, MIN_PROTO_MAJOR};

/// Longest line accepted. gpsd truncates its own responses at 10240 bytes, so a longer line does
/// not come from gpsd.
pub(crate) const MAX_LINE_LEN: usize = 64 * 1024;

/// One line from gpsd.
#[derive(Debug)]
pub(crate) enum Line {
    /// A JSON report in the gpsd format.
    Json(Report),
    /// An NMEA sentence, as gpsd sent it, line ending included.
    Nmea(String),
}

pub(crate) struct Connection {
    reader: BufReader<TcpStream>,
    /// The line being read. A read timeout leaves a partial line here, and the next read appends
    /// the rest.
    line: Vec<u8>,
    peer: SocketAddr,
}

#[derive(Debug)]
pub(crate) enum ConnectError {
    /// The WATCH request could not be built; nothing was sent.
    Request(RequestTooLong),
    Io(io::Error),
}

impl From<io::Error> for ConnectError {
    fn from(e: io::Error) -> ConnectError {
        ConnectError::Io(e)
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectError::Request(e) => write!(f, "{e}"),
            ConnectError::Io(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Debug)]
pub(crate) enum ReadError {
    /// No complete line arrived within the read timeout. The connection is still usable, and a
    /// partial line is kept for the next read.
    Timeout,
    /// gpsd closed the connection.
    Closed,
    Io(io::Error),
    /// A complete line that is neither a JSON report nor an NMEA sentence. The connection is still
    /// usable.
    Parse {
        line: String,
        error: serde_json::Error,
    },
    /// A line longer than [`MAX_LINE_LEN`] with no end in sight. The connection should be dropped.
    LineTooLong,
    /// gpsd is older than the JSON protocol.
    UnsupportedProtocol(Version),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Timeout => write!(f, "no complete line within the read timeout"),
            ReadError::Closed => write!(f, "gpsd closed the connection"),
            ReadError::Io(e) => write!(f, "{e}"),
            ReadError::Parse { line, error } => write!(f, "not a report ({error}): {line}"),
            ReadError::LineTooLong => write!(f, "line longer than {MAX_LINE_LEN} bytes"),
            ReadError::UnsupportedProtocol(v) => write!(
                f,
                "gpsd {} speaks protocol {}.{}; {MIN_PROTO_MAJOR} or later is needed",
                v.release, v.proto_major, v.proto_minor
            ),
        }
    }
}

impl Connection {
    /// Connects and sends the WATCH request. gpsd's VERSION banner and its answers to the request
    /// come back through [`Connection::read`], like every other report, so the caller sees the
    /// device list.
    pub(crate) fn connect(
        addr: SocketAddr,
        watch: &Watch,
        connect_timeout: Duration,
        read_timeout: Duration,
    ) -> Result<Connection, ConnectError> {
        let request = watch.request().map_err(ConnectError::Request)?;
        let mut stream = TcpStream::connect_timeout(&addr, connect_timeout)?;
        stream.set_read_timeout(Some(read_timeout))?;
        stream.set_write_timeout(Some(connect_timeout))?;
        stream.write_all(request.as_bytes())?;
        Ok(Connection {
            reader: BufReader::new(stream),
            line: Vec::new(),
            peer: addr,
        })
    }

    pub(crate) fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// The next report or NMEA sentence. Blank lines are skipped. A line that begins with `$` is an
    /// NMEA sentence, and is not checked further. A VERSION report with a protocol major version
    /// below [`MIN_PROTO_MAJOR`] is an error.
    pub(crate) fn read(&mut self) -> Result<Line, ReadError> {
        loop {
            let line = self.read_line()?;
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            if line.first() == Some(&b'$') {
                return Ok(Line::Nmea(String::from_utf8_lossy(&line).into_owned()));
            }
            return match serde_json::from_slice::<Report>(&line) {
                Ok(Report::Version(v)) if v.proto_major < MIN_PROTO_MAJOR => {
                    Err(ReadError::UnsupportedProtocol(v))
                }
                Ok(report) => Ok(Line::Json(report)),
                Err(error) => Err(ReadError::Parse {
                    line: String::from_utf8_lossy(&line).trim_end().to_owned(),
                    error,
                }),
            };
        }
    }

    /// One complete line, newline included.
    fn read_line(&mut self) -> Result<Vec<u8>, ReadError> {
        // `take` caps the line. read_until returns Ok without a newline only at the cap or at the end of
        // the stream.
        let budget = (MAX_LINE_LEN + 1 - self.line.len()) as u64;
        match (&mut self.reader).take(budget).read_until(b'\n', &mut self.line) {
            Ok(_) => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                return Err(ReadError::Timeout);
            }
            Err(e) => return Err(ReadError::Io(e)),
        }
        if self.line.last() != Some(&b'\n') {
            let partial = std::mem::take(&mut self.line);
            return Err(if partial.len() > MAX_LINE_LEN {
                ReadError::LineTooLong
            } else {
                ReadError::Closed
            });
        }
        Ok(std::mem::take(&mut self.line))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::{
        super::{
            protocol::{tests::samples::*, Mode},
            tests::{line, FakeGpsd, Step},
        },
        *,
    };

    const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
    const READ_TIMEOUT: Duration = Duration::from_millis(100);

    fn connect(fake: &FakeGpsd, watch: &Watch) -> Connection {
        Connection::connect(fake.addr, watch, CONNECT_TIMEOUT, READ_TIMEOUT).unwrap()
    }

    /// Reads past timeouts until something else happens.
    fn read(conn: &mut Connection) -> Result<Line, ReadError> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match conn.read() {
                Err(ReadError::Timeout) if Instant::now() < deadline => {}
                other => return other,
            }
        }
    }

    #[test]
    fn handshake_in_gpsd_order() {
        let fake = FakeGpsd::start(vec![vec![
            line(VERSION),
            Step::ExpectRequest("?WATCH="),
            line(DEVICES),
            line(WATCH),
            line(TPV_3D),
            Step::WaitForClose,
        ]]);
        let mut conn = connect(&fake, &Watch::default());
        assert_eq!(conn.peer(), fake.addr);
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Version(v))) if v.release == "3.25"));
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Devices(d))) if d.devices.len() == 1));
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Watch(w))) if w.enable && w.json));
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Tpv(t))) if t.mode == Mode::Fix3d));
        assert_eq!(
            fake.requests(),
            vec!["?WATCH={\"enable\":true,\"json\":true,\"nmea\":true};\n".to_owned()]
        );
    }

    #[test]
    fn partial_line_survives_a_read_timeout() {
        let (head, tail) = TPV_3D.split_at(TPV_3D.len() / 2);
        let fake = FakeGpsd::start(vec![vec![
            line(VERSION),
            Step::ExpectRequest("?WATCH="),
            Step::Send(head.to_owned()),
            Step::Sleep(READ_TIMEOUT * 3),
            line(tail),
            Step::WaitForClose,
        ]]);
        let mut conn = connect(&fake, &Watch::default());
        assert!(matches!(conn.read(), Ok(Line::Json(Report::Version(_)))));
        let mut timeouts = 0;
        let report = loop {
            match conn.read() {
                Err(ReadError::Timeout) => timeouts += 1,
                other => break other,
            }
            assert!(timeouts < 50, "the second half never arrived");
        };
        assert!(timeouts >= 1, "the first half should have arrived alone");
        let Ok(Line::Json(Report::Tpv(tpv))) = report else {
            panic!("expected the whole TPV, got {report:?}")
        };
        assert_eq!((tpv.lat, tpv.eph), (Some(44.069009667), Some(17.29)));
    }

    #[test]
    fn nmea_sentences_are_passed_on_as_sent() {
        let fake = FakeGpsd::start(vec![vec![
            line(VERSION),
            Step::ExpectRequest("?WATCH="),
            Step::Send(format!("{GGA}\r\n")),
            line(TPV_3D),
            Step::WaitForClose,
        ]]);
        let mut conn = connect(&fake, &Watch::default());
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Version(_)))));
        assert!(matches!(read(&mut conn), Ok(Line::Nmea(s)) if s == format!("{GGA}\r\n")));
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Tpv(_)))));
    }

    #[test]
    fn unknown_class_and_bad_line_do_not_break_the_stream() {
        let fake = FakeGpsd::start(vec![vec![
            line(VERSION),
            Step::ExpectRequest("?WATCH="),
            line(TOFF),
            line("{this is not json"),
            line(""),
            line(TPV_NO_FIX),
            Step::WaitForClose,
        ]]);
        let mut conn = connect(&fake, &Watch::default());
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Version(_)))));
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Unknown))));
        assert!(matches!(
            read(&mut conn),
            Err(ReadError::Parse { line, .. }) if line == "{this is not json"
        ));
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Tpv(t))) if t.mode == Mode::NoFix));
    }

    #[test]
    fn server_close_is_reported_as_closed() {
        let fake = FakeGpsd::start(vec![vec![
            line(VERSION),
            Step::ExpectRequest("?WATCH="),
            Step::Send(r#"{"class":"TPV","mode":1"#.to_owned()),
            Step::Close,
        ]]);
        let mut conn = connect(&fake, &Watch::default());
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Version(_)))));
        // The unfinished line is discarded with the connection.
        assert!(matches!(read(&mut conn), Err(ReadError::Closed)));
        assert!(matches!(read(&mut conn), Err(ReadError::Closed)));
    }

    #[test]
    fn overlong_line_is_an_error() {
        let fake = FakeGpsd::start(vec![vec![
            line(VERSION),
            Step::ExpectRequest("?WATCH="),
            Step::Send("x".repeat(MAX_LINE_LEN + 1000)),
            Step::WaitForClose,
        ]]);
        let mut conn = connect(&fake, &Watch::default());
        assert!(matches!(read(&mut conn), Ok(Line::Json(Report::Version(_)))));
        assert!(matches!(read(&mut conn), Err(ReadError::LineTooLong)));
    }

    #[test]
    fn old_protocol_is_refused() {
        let fake = FakeGpsd::start(vec![vec![
            line(VERSION_OLD),
            Step::ExpectRequest("?WATCH="),
            Step::WaitForClose,
        ]]);
        let mut conn = connect(&fake, &Watch::default());
        assert!(matches!(
            read(&mut conn),
            Err(ReadError::UnsupportedProtocol(v)) if v.proto_major == 2
        ));
    }

    #[test]
    fn overlong_request_fails_before_connecting() {
        let fake = FakeGpsd::start(vec![]);
        let watch = Watch {
            device: Some(format!("/dev/{}", "d".repeat(100))),
            ..Watch::default()
        };
        let result = Connection::connect(fake.addr, &watch, CONNECT_TIMEOUT, READ_TIMEOUT);
        assert!(matches!(result, Err(ConnectError::Request(RequestTooLong(_)))));
        assert!(!fake.wait_closed(1, Duration::from_millis(200)));
    }
}
