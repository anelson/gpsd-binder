//! gpsd's JSON reports and the WATCH request, as Rust types.
//!
//! Names and meanings follow `gpsd_json(5)`. Every field the protocol marks optional is an
//! `Option`. Unknown fields are ignored, because gpsd adds fields between releases. Enumerated
//! values keep an unknown number in a fallback variant, so a new value never discards a whole
//! report.
//!
//! Only the classes and fields the driver uses are modelled; add fields as they are needed. Any
//! other class, such as TOFF or PPS, deserializes to [`Report::Unknown`].

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One line from gpsd, told apart by its `class` field.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "class", rename_all = "UPPERCASE")]
pub(crate) enum Report {
    Version(Version),
    Devices(Devices),
    Device(Device),
    Watch(Watch),
    Tpv(Tpv),
    Sky(Sky),
    Error(ErrorReport),
    /// A class this module does not model.
    #[serde(other)]
    Unknown,
}

/// gpsd sends this first on every connection.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Version {
    pub(crate) release: String,
    pub(crate) proto_major: u32,
    pub(crate) proto_minor: u32,
}

/// The JSON protocol exists from protocol version 3.
pub(crate) const MIN_PROTO_MAJOR: u32 = 3;

/// The device list, sent in answer to a WATCH request.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Devices {
    pub(crate) devices: Vec<Device>,
}

/// One receiver. Also sent on its own when a device is activated.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Device {
    pub(crate) path: Option<String>,
    pub(crate) driver: Option<String>,
    pub(crate) subtype: Option<String>,
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path.as_deref().unwrap_or("(unnamed device)"))?;
        if let Some(driver) = &self.driver {
            write!(f, ", driver {driver}")?;
        }
        if let Some(subtype) = &self.subtype {
            write!(f, " ({subtype})")?;
        }
        Ok(())
    }
}

/// The WATCH request, and gpsd's echo of the watch settings it applied.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Watch {
    #[serde(default)]
    pub(crate) enable: bool,
    #[serde(default)]
    pub(crate) json: bool,
    /// NMEA sentences as well, each on a line of its own, as received or as gpsd builds them from
    /// a binary protocol.
    #[serde(default)]
    pub(crate) nmea: bool,
    /// Watch only this device; otherwise every device gpsd has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) device: Option<String>,
}

impl Default for Watch {
    /// Every device, JSON reports and NMEA sentences.
    fn default() -> Watch {
        Watch {
            enable: true,
            json: true,
            nmea: true,
            device: None,
        }
    }
}

/// gpsd reads at most this many characters of a request line, newline excluded.
pub(crate) const MAX_REQUEST_LEN: usize = 80;

/// The WATCH request line would exceed [`MAX_REQUEST_LEN`]; a long device path is the usual cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestTooLong(pub(crate) usize);

impl fmt::Display for RequestTooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "WATCH request is {} characters; gpsd reads at most {MAX_REQUEST_LEN}",
            self.0
        )
    }
}

impl Watch {
    /// The request line for these settings, newline included.
    pub(crate) fn request(&self) -> Result<String, RequestTooLong> {
        let json = serde_json::to_string(self).expect("booleans and strings always serialize");
        let line = format!("?WATCH={json};");
        if line.len() > MAX_REQUEST_LEN {
            return Err(RequestTooLong(line.len()));
        }
        Ok(format!("{line}\n"))
    }
}

/// A time-position-velocity report: one fix, or the lack of one.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub(crate) struct Tpv {
    pub(crate) mode: Mode,
    pub(crate) time: Option<DateTime<Utc>>,
    pub(crate) lat: Option<f64>,
    pub(crate) lon: Option<f64>,
    /// Height above the WGS84 ellipsoid, meters. `altMSL` and the deprecated `alt` are not it.
    #[serde(rename = "altHAE")]
    pub(crate) alt_hae: Option<f64>,
    /// Horizontal position error, meters.
    pub(crate) eph: Option<f64>,
    /// Vertical position error, meters.
    pub(crate) epv: Option<f64>,
    /// Course over ground, degrees from true north.
    pub(crate) track: Option<f64>,
    /// Speed over ground, meters per second.
    pub(crate) speed: Option<f64>,
    /// Track and speed error estimates.
    pub(crate) epd: Option<f64>,
    pub(crate) eps: Option<f64>,
}

/// TPV `mode`.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(from = "u8")]
pub(crate) enum Mode {
    Unknown,
    NoFix,
    Fix2d,
    Fix3d,
    /// A value the protocol does not define.
    Other,
}

impl From<u8> for Mode {
    fn from(n: u8) -> Mode {
        match n {
            0 => Mode::Unknown,
            1 => Mode::NoFix,
            2 => Mode::Fix2d,
            3 => Mode::Fix3d,
            _ => Mode::Other,
        }
    }
}

impl Mode {
    pub(crate) fn has_fix(self) -> bool {
        matches!(self, Mode::Fix2d | Mode::Fix3d)
    }
}

/// A sky view. gpsd also sends SKY reports with only the dilution-of-precision values and no
/// satellite list.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub(crate) struct Sky {
    pub(crate) satellites: Option<Vec<Satellite>>,
}

/// One satellite in a SKY report
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub(crate) struct Satellite {
    /// gpsd's single number over all constellations. The protocol marks it always present; it is
    /// optional here so that one odd satellite cannot drop a whole SKY report.
    #[serde(rename = "PRN")]
    prn: Option<i32>,
    /// gpsd sends `gnssid` and `svid` together, but some drivers send only one of them.
    pub(crate) gnssid: Option<GnssId>,
    /// Satellite number within its constellation.
    pub(crate) svid: Option<u16>,
    /// GLONASS only: the frequency channel plus 7.
    freqid: Option<u8>,
    /// Elevation and azimuth, degrees.
    pub(crate) el: Option<f64>,
    pub(crate) az: Option<f64>,
    /// Signal strength, dB-Hz.
    pub(crate) ss: Option<f64>,
    #[serde(default)]
    pub(crate) used: bool,
}

/// Satellite `gnssid`: the constellation, in u-blox numbering.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(from = "u8")]
pub(crate) enum GnssId {
    Gps,
    Sbas,
    Galileo,
    BeiDou,
    Imes,
    Qzss,
    Glonass,
    NavIc,
    /// A value the protocol does not define.
    Other,
}

impl From<u8> for GnssId {
    fn from(n: u8) -> GnssId {
        match n {
            0 => GnssId::Gps,
            1 => GnssId::Sbas,
            2 => GnssId::Galileo,
            3 => GnssId::BeiDou,
            4 => GnssId::Imes,
            5 => GnssId::Qzss,
            6 => GnssId::Glonass,
            // gpsd's Skytraq driver writes 20.
            7 | 20 => GnssId::NavIc,
            _ => GnssId::Other,
        }
    }
}

/// A satellite's constellation and its number there, as gpsd's `gnssid` and `svid` number them: the
/// PRN, except for QZSS (the PRN less 192), IMES (less 172) and GLONASS (the orbital slot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SatelliteId {
    Svid(GnssId, i32),
    /// A GLONASS satellite whose slot is unknown, by its frequency channel, -7 to 6.
    GlonassChannel(i32),
}

impl Satellite {
    /// Which satellite this is: from `gnssid` and `svid` when gpsd sends them, otherwise from
    /// `PRN`. None in the hopefully-impossible case that neither is available.
    pub(crate) fn id(&self) -> Option<SatelliteId> {
        let (Some(gnssid), Some(svid)) = (self.gnssid, self.svid) else {
            return SatelliteId::from_prn(self.prn?);
        };
        let svid = i32::from(svid);
        // Some of gpsd's drivers depart from its numbering.
        Some(match (gnssid, svid) {
            // GREIS: SBAS 120-142 as 1-23.
            (GnssId::Sbas, 1..=23) => SatelliteId::Svid(GnssId::Sbas, svid + 119),
            // TSIP: SBAS 120-138 from a Trimble Copernicus as 207-225.
            (GnssId::Sbas, 207..=225) => SatelliteId::Svid(GnssId::Sbas, svid - 87),
            // SiRF: QZSS's SBAS signal (SAIF, PRN 183-187) as GPS.
            (GnssId::Gps, 183..=187) => SatelliteId::Svid(GnssId::Sbas, svid),
            // Allystar: GLONASS slots as 65-96.
            (GnssId::Glonass, 65..=96) => SatelliteId::Svid(GnssId::Glonass, svid - 64),
            // TSIP: some QZSS satellites by PRN.
            (GnssId::Qzss, 193..=202) => SatelliteId::Svid(GnssId::Qzss, svid - 192),
            // The slot is unknown; the frequency channel identifies the satellite, if gpsd sends it.
            (GnssId::Glonass, 255) => SatelliteId::GlonassChannel(i32::from(self.freqid?) - 7),
            (GnssId::Other, _) => return None,
            _ => SatelliteId::Svid(gnssid, svid),
        })
    }
}

impl SatelliteId {
    /// Decodes gpsd's single number over all constellations, as its NMEA and u-blox drivers make
    /// it.
    fn from_prn(prn: i32) -> Option<SatelliteId> {
        let (gnssid, svid) = match prn {
            1..=32 => (GnssId::Gps, prn),
            33..=64 => (GnssId::Sbas, prn + 87),
            65..=96 => (GnssId::Glonass, prn - 64),
            120..=158 => (GnssId::Sbas, prn),
            173..=182 => (GnssId::Imes, prn - 172),
            193..=202 => (GnssId::Qzss, prn - 192),
            301..=336 => (GnssId::Galileo, prn - 300),
            401..=463 => (GnssId::BeiDou, prn - 400),
            501..=518 => (GnssId::NavIc, prn - 500),
            801..=814 => (GnssId::NavIc, prn - 800),
            _ => return None,
        };
        Some(SatelliteId::Svid(gnssid, svid))
    }
}

/// gpsd's answer to a request it cannot carry out.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ErrorReport {
    pub(crate) message: String,
}

#[cfg(test)]
pub(crate) mod tests {
    use self::samples::*;
    use super::*;

    fn parse(line: &str) -> Report {
        serde_json::from_str(line).unwrap_or_else(|e| panic!("{line}: {e}"))
    }

    #[test]
    fn tpv_with_a_3d_fix() {
        let Report::Tpv(tpv) = parse(TPV_3D) else {
            panic!("not TPV")
        };
        assert_eq!(tpv.mode, Mode::Fix3d);
        assert!(tpv.mode.has_fix());
        assert_eq!(tpv.time.unwrap().timestamp_millis(), 2_046_014_367_000);
        assert_eq!((tpv.lat, tpv.lon), (Some(44.069009667), Some(-121.314245833)));
        assert_eq!(tpv.alt_hae, Some(1125.7));
        assert_eq!((tpv.eph, tpv.epv), (Some(17.29), Some(29.21)));
        assert_eq!((tpv.track, tpv.speed), (Some(12.5), Some(0.043)));
        assert_eq!((tpv.epd, tpv.eps), (Some(1.5), Some(21.89)));
    }

    #[test]
    fn tpv_without_a_fix_has_no_position() {
        let Report::Tpv(tpv) = parse(TPV_NO_FIX) else {
            panic!("not TPV")
        };
        assert_eq!(tpv.mode, Mode::NoFix);
        assert!(!tpv.mode.has_fix());
        assert_eq!((tpv.lat, tpv.lon, tpv.time), (None, None, None));
    }

    #[test]
    fn unknown_numbers_and_fields_are_tolerated() {
        let Report::Tpv(tpv) = parse(TPV_ODD) else {
            panic!("not TPV")
        };
        assert_eq!(tpv.mode, Mode::Other);
        assert!(!tpv.mode.has_fix());
        assert_eq!(tpv.lat, Some(1.0));
    }

    #[test]
    fn sky_with_satellites_from_three_constellations() {
        let Report::Sky(sky) = parse(SKY) else {
            panic!("not SKY")
        };
        let sats = sky.satellites.as_deref().unwrap();
        assert_eq!(sats.len(), 4);
        let gps = &sats[0];
        assert_eq!(
            (gps.prn, gps.gnssid, gps.svid),
            (Some(1), Some(GnssId::Gps), Some(1))
        );
        assert_eq!((gps.el, gps.az, gps.ss), (Some(53.0), Some(280.0), Some(36.0)));
        assert!(gps.used);
        let glonass = &sats[1];
        assert_eq!(
            (glonass.prn, glonass.gnssid, glonass.svid),
            (Some(67), Some(GnssId::Glonass), Some(3))
        );
        assert_eq!(sats[2].gnssid, Some(GnssId::Galileo));
        let odd = &sats[3];
        assert_eq!(odd.gnssid, Some(GnssId::Other));
        assert_eq!((odd.el, odd.az, odd.ss), (None, None, None));
        assert!(!odd.used);
    }

    #[test]
    fn sky_with_dop_values_only() {
        let Report::Sky(sky) = parse(SKY_DOPS_ONLY) else {
            panic!("not SKY")
        };
        assert_eq!(sky.satellites, None);
    }

    #[test]
    fn satellite_ids() {
        let id = |sat: &str| serde_json::from_str::<Satellite>(sat).unwrap().id();
        for (sat, gnssid, svid) in [
            (r#"{"PRN":303,"gnssid":2,"svid":3}"#, GnssId::Galileo, 3),
            (r#"{"PRN":131,"gnssid":1,"svid":12}"#, GnssId::Sbas, 131),
            (r#"{"PRN":138,"gnssid":1,"svid":225}"#, GnssId::Sbas, 138),
            (r#"{"PRN":183,"gnssid":0,"svid":183}"#, GnssId::Sbas, 183),
            (r#"{"PRN":65,"gnssid":6,"svid":65}"#, GnssId::Glonass, 1),
            (r#"{"PRN":194,"gnssid":5,"svid":194}"#, GnssId::Qzss, 2),
            (r#"{"PRN":241,"gnssid":20,"svid":1}"#, GnssId::NavIc, 1),
        ] {
            assert_eq!(id(sat), Some(SatelliteId::Svid(gnssid, svid)), "{sat}");
        }
        let glonass = r#"{"PRN":-1,"gnssid":6,"svid":255,"freqid":0}"#;
        assert_eq!(id(glonass), Some(SatelliteId::GlonassChannel(-7)));
        for unknown in [
            r#"{"PRN":70,"gnssid":6,"svid":255}"#,
            r#"{"PRN":12,"gnssid":9,"svid":12}"#,
        ] {
            assert_eq!(id(unknown), None, "{unknown}");
        }

        // PRN alone: the first and last number of each range, and numbers outside them.
        let prn = |prn: i32| id(&format!(r#"{{"PRN":{prn}}}"#));
        for (prns, gnssid, svids) in [
            ((1, 32), GnssId::Gps, (1, 32)),
            ((33, 64), GnssId::Sbas, (120, 151)),
            ((65, 96), GnssId::Glonass, (1, 32)),
            ((120, 158), GnssId::Sbas, (120, 158)),
            ((173, 182), GnssId::Imes, (1, 10)),
            ((193, 202), GnssId::Qzss, (1, 10)),
            ((301, 336), GnssId::Galileo, (1, 36)),
            ((401, 463), GnssId::BeiDou, (1, 63)),
            ((501, 518), GnssId::NavIc, (1, 18)),
            ((801, 814), GnssId::NavIc, (1, 14)),
        ] {
            let svid = |n| Some(SatelliteId::Svid(gnssid, n));
            assert_eq!(
                (prn(prns.0), prn(prns.1)),
                (svid(svids.0), svid(svids.1)),
                "PRN {prns:?}"
            );
        }
        for unknown in [-1, 0, 97, 119, 159, 183, 203, 300, 999] {
            assert_eq!(prn(unknown), None, "PRN {unknown}");
        }
    }

    #[test]
    fn handshake_classes() {
        let Report::Version(v) = parse(VERSION) else {
            panic!("not VERSION")
        };
        assert_eq!(
            (v.release.as_str(), v.proto_major, v.proto_minor),
            ("3.25", 3, 15)
        );

        // gpsd 3.25 writes "readonly":"true" as a string, which the specification does not allow; an
        // unknown field does not break the device list.
        let Report::Devices(d) = parse(DEVICES) else {
            panic!("not DEVICES")
        };
        assert_eq!(d.devices.len(), 1);
        assert_eq!(d.devices[0].to_string(), "/dev/pts/1, driver NMEA0183");

        let Report::Device(d) = parse(DEVICE) else {
            panic!("not DEVICE")
        };
        assert_eq!(
            d.to_string(),
            "/dev/pts/1, driver u-blox (SW EXT CORE 3.01 (107900),HW 00080000)"
        );

        let Report::Watch(w) = parse(WATCH) else {
            panic!("not WATCH")
        };
        assert_eq!(w, Watch::default());
        let Report::Watch(w) = parse(WATCH_REFUSED) else {
            panic!("not WATCH")
        };
        assert!(!w.enable && !w.json);

        let Report::Error(e) = parse(ERROR) else {
            panic!("not ERROR")
        };
        assert_eq!(e.message, "No such device as /dev/pts/0");
    }

    #[test]
    fn unknown_class_is_unknown_not_an_error() {
        assert_eq!(parse(TOFF), Report::Unknown);
    }

    #[test]
    fn lines_that_are_not_reports_fail() {
        for line in [
            r#"{"device":"/dev/pts/1","mode":3}"#,
            r#"{"class":"TPV","device":"/dev/pts/1"}"#,
            "[1,2,3]",
        ] {
            serde_json::from_str::<Report>(line).unwrap_err();
        }
    }

    #[test]
    fn watch_request() {
        assert_eq!(
            Watch::default().request().unwrap(),
            "?WATCH={\"enable\":true,\"json\":true,\"nmea\":true};\n"
        );
        let one_device = Watch {
            device: Some("/dev/ttyACM0".to_string()),
            ..Watch::default()
        };
        assert_eq!(
            one_device.request().unwrap(),
            "?WATCH={\"enable\":true,\"json\":true,\"nmea\":true,\"device\":\"/dev/ttyACM0\"};\n"
        );
        let long_path = Watch {
            device: Some(format!("/dev/serial/by-id/{}", "x".repeat(60))),
            ..Watch::default()
        };
        assert!(matches!(long_path.request(), Err(RequestTooLong(n)) if n > MAX_REQUEST_LEN));
    }

    /// Lines as gpsd 3.25 writes them, for the tests of this module, the connection, the feed and
    /// the HAL. The device is gpsfake's pty.
    pub(crate) mod samples {
        pub(crate) const VERSION: &str =
            r#"{"class":"VERSION","release":"3.25","rev":"3.25","proto_major":3,"proto_minor":15}"#;
        pub(crate) const VERSION_OLD: &str =
            r#"{"class":"VERSION","release":"2.39","rev":"2.39","proto_major":2,"proto_minor":0}"#;
        pub(crate) const DEVICES: &str = concat!(
            r#"{"class":"DEVICES","devices":[{"class":"DEVICE","path":"/dev/pts/1","driver":"NMEA0183","#,
            r#""readonly":"true","activated":"2026-10-24T23:28:22.095Z","flags":1,"native":0,"#,
            r#""bps":4800,"parity":"N","stopbits":1,"cycle":1.00}]}"#
        );
        pub(crate) const DEVICE: &str = concat!(
            r#"{"class":"DEVICE","path":"/dev/pts/1","driver":"u-blox","#,
            r#""subtype":"SW EXT CORE 3.01 (107900),HW 00080000","activated":"2026-10-24T23:28:22.095Z","#,
            r#""flags":1,"native":1,"bps":9600,"parity":"N","stopbits":1,"cycle":1.00,"mincycle":0.25}"#
        );
        pub(crate) const WATCH: &str = concat!(
            r#"{"class":"WATCH","enable":true,"json":true,"nmea":true,"raw":0,"scaled":false,"#,
            r#""timing":false,"split24":false,"pps":false}"#
        );
        pub(crate) const WATCH_REFUSED: &str = concat!(
            r#"{"class":"WATCH","enable":false,"json":false,"nmea":false,"raw":0,"scaled":false,"#,
            r#""timing":false,"split24":false,"pps":false}"#
        );
        pub(crate) const TPV_3D: &str = concat!(
            r#"{"class":"TPV","device":"/dev/pts/1","mode":3,"status":2,"#,
            r#""time":"2034-11-01T17:19:27.000Z","ept":0.005,"leapseconds":18,"#,
            r#""lat":44.069009667,"lon":-121.314245833,"altHAE":1125.7000,"altMSL":1147.0000,"#,
            r#""alt":1147.0000,"epx":6.294,"epy":10.947,"epv":29.210,"magvar":14.4,"track":12.5,"#,
            r#""speed":0.043,"climb":-0.200,"epd":1.5,"eps":21.89,"epc":58.42,"geoidSep":-21.300,"#,
            r#""eph":17.290,"sep":29.640}"#
        );
        pub(crate) const TPV_NO_FIX: &str = r#"{"class":"TPV","device":"/dev/pts/1","mode":1}"#;
        pub(crate) const TPV_ODD: &str = concat!(
            r#"{"class":"TPV","device":"/dev/pts/1","mode":7,"status":42,"lat":1.0,"lon":2.0,"#,
            r#""jam":3,"newfield":{"x":[1,2]}}"#
        );
        pub(crate) const SKY: &str = concat!(
            r#"{"class":"SKY","device":"/dev/pts/1","time":"2034-11-01T17:19:27.000Z","gdop":1.68,"#,
            r#""hdop":0.91,"pdop":1.56,"tdop":0.81,"xdop":0.42,"ydop":0.73,"vdop":1.27,"nSat":4,"#,
            r#""uSat":3,"satellites":["#,
            r#"{"PRN":1,"gnssid":0,"svid":1,"sigid":0,"health":1,"az":280.0,"el":53.0,"ss":36.0,"#,
            r#""used":true},"#,
            r#"{"PRN":67,"gnssid":6,"svid":3,"freqid":12,"az":349.0,"el":62.0,"ss":26.0,"#,
            r#""used":true,"prRes":1.5},"#,
            r#"{"PRN":303,"gnssid":2,"svid":3,"sigid":1,"az":101.0,"el":20.0,"ss":31.0,"used":true},"#,
            r#"{"PRN":12,"gnssid":9,"svid":12,"health":5,"used":false}]}"#
        );
        pub(crate) const SKY_DOPS_ONLY: &str =
            r#"{"class":"SKY","device":"/dev/pts/1","hdop":0.91,"pdop":1.56,"vdop":1.27,"uSat":12}"#;
        pub(crate) const ERROR: &str = r#"{"class":"ERROR","message":"No such device as /dev/pts/0"}"#;
        /// An NMEA sentence from the recording that the container test replays, without its line
        /// ending.
        pub(crate) const GGA: &str =
            "$GNGGA,171926.00,4404.14063,N,12118.85478,W,1,12,0.91,1147.2,M,-21.3,M,,*44";
        pub(crate) const TOFF: &str = concat!(
            r#"{"class":"TOFF","device":"/dev/pts/1","real_sec":1567164286,"real_nsec":0,"#,
            r#""clock_sec":1567164286,"clock_nsec":241446000}"#
        );
    }
}
