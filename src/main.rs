//! gpsd-binder: an android.hardware.gnss AIDL (version 2) driver written in
//! pure Rust with rsbinder, built outside the Android tree.
//!
//! STATUS: prototype. This is the code of the 2026-10-24 experiment that
//! proved the approach on Waydroid (LineageOS 20, Android 13, x86_64); see
//! transfer.md. It is not yet a finished driver.
//!
//! It registers android.hardware.gnss.IGnss/default with servicemanager. When
//! Android starts a GNSS session it connects to gpsd (service.gpsd.host and
//! service.gpsd.port, default 127.0.0.1:2947), and passes gpsd's satellite
//! list (SKY) and fixes (TPV) to Android through IGnssCallback. Every optional
//! extension answers "unsupported operation", which Android records as absent.
//! Log lines go to stderr; the init file sends them to the kernel log
//! (stdio_to_kmsg), one record per write.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rsbinder::*;

#[allow(clippy::all, non_snake_case, non_upper_case_globals, unused)]
mod gen {
    include!(concat!(env!("OUT_DIR"), "/gnss_aidl.rs"));
}

use g::ElapsedRealtime::ElapsedRealtime;
use g::GnssConstellationType::GnssConstellationType as Constellation;
use g::GnssLocation::GnssLocation;
use g::IGnss::{
    BnGnss, GnssAidingData::GnssAidingData, IGnss, PositionModeOptions::PositionModeOptions,
};
use g::IGnssCallback::{
    GnssStatusValue::GnssStatusValue, GnssSvInfo::GnssSvInfo, GnssSystemInfo::GnssSystemInfo,
    IGnssCallback,
};
use gen::android::hardware::gnss as g;

const SERVICE_NAME: &str = "android.hardware.gnss.IGnss/default";

// One write() per line: init's stdio_to_kmsg turns every write into a
// separate kernel log record, and eprintln! issues several writes per line.
macro_rules! log {
    ($($arg:tt)*) => {{
        let line = format!("gpsd-binder: {}\n", format!($($arg)*));
        let _ = std::io::stderr().lock().write_all(line.as_bytes());
    }};
}

type Callback = Arc<Mutex<Option<Strong<dyn IGnssCallback>>>>;

struct Gnss {
    callback: Callback,
    // Each start() bumps the session number; a worker runs only while its
    // number is current, so stop() and a later start() never overlap.
    session: Arc<AtomicU64>,
}

fn unsupported<T>() -> BinderResult<T> {
    Err(ExceptionCode::UnsupportedOperation.into())
}

fn getprop(name: &str, default: &str) -> String {
    std::process::Command::new("/system/bin/getprop")
        .arg(name)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn boottime_ns() -> i64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    ts.tv_sec as i64 * 1_000_000_000 + ts.tv_nsec as i64
}

/// Milliseconds since 1970 for an ISO 8601 UTC time such as "2026-10-24T20:48:31.250Z".
fn iso8601_ms(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let mut t = time.split(':');
    let (hh, mm) = (
        t.next()?.parse::<i64>().ok()?,
        t.next()?.parse::<i64>().ok()?,
    );
    let secs: f64 = t.next()?.parse().ok()?;
    // Days from 1970-01-01 (H. Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(((days * 86400 + hh * 3600 + mm * 60) as f64 * 1000.0 + secs * 1000.0).round() as i64)
}

fn constellation(gnssid: i64) -> Constellation {
    match gnssid {
        0 => Constellation::GPS,
        1 => Constellation::SBAS,
        2 => Constellation::GALILEO,
        3 => Constellation::BEIDOU,
        5 => Constellation::QZSS,
        6 => Constellation::GLONASS,
        7 => Constellation::IRNSS,
        _ => Constellation::UNKNOWN,
    }
}

fn sky_to_svs(v: &serde_json::Value) -> Option<Vec<GnssSvInfo>> {
    let sats = v.get("satellites")?.as_array()?;
    Some(
        sats.iter()
            .map(|s| {
                let f = |k: &str| s.get(k).and_then(|x| x.as_f64());
                let used = s.get("used").and_then(|x| x.as_bool()).unwrap_or(false);
                GnssSvInfo {
                    svid: f("svid").or(f("PRN")).unwrap_or(0.0) as i32,
                    constellation: constellation(f("gnssid").unwrap_or(-1.0) as i64),
                    cN0Dbhz: f("ss").unwrap_or(0.0) as f32,
                    basebandCN0DbHz: 0.0,
                    elevationDegrees: f("el").unwrap_or(0.0) as f32,
                    azimuthDegrees: f("az").unwrap_or(0.0) as f32,
                    carrierFrequencyHz: 0,
                    svFlag: if used { 4 } else { 0 }, // GnssSvFlags::USED_IN_FIX
                }
            })
            .collect(),
    )
}

fn tpv_to_location(v: &serde_json::Value) -> Option<GnssLocation> {
    let f = |k: &str| v.get(k).and_then(|x| x.as_f64());
    if f("mode").unwrap_or(0.0) < 2.0 {
        return None;
    }
    let mut loc = GnssLocation {
        gnssLocationFlags: g::GnssLocation::HAS_LAT_LONG,
        latitudeDegrees: f("lat")?,
        longitudeDegrees: f("lon")?,
        timestampMillis: iso8601_ms(v.get("time")?.as_str()?)?,
        elapsedRealtime: ElapsedRealtime {
            flags: g::ElapsedRealtime::HAS_TIMESTAMP_NS,
            timestampNs: boottime_ns(),
            timeUncertaintyNs: 0.0,
        },
        ..Default::default()
    };
    let mut add = |flag: i32, value: Option<f64>, slot: &mut f64| {
        if let Some(x) = value {
            loc.gnssLocationFlags |= flag;
            *slot = x;
        }
    };
    let (mut alt, mut spd, mut brg, mut hacc, mut vacc, mut sacc, mut bacc) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    add(
        g::GnssLocation::HAS_ALTITUDE,
        f("altHAE").or(f("alt")),
        &mut alt,
    );
    add(g::GnssLocation::HAS_SPEED, f("speed"), &mut spd);
    add(g::GnssLocation::HAS_BEARING, f("track"), &mut brg);
    add(
        g::GnssLocation::HAS_HORIZONTAL_ACCURACY,
        f("eph").or(f("epx")),
        &mut hacc,
    );
    add(g::GnssLocation::HAS_VERTICAL_ACCURACY, f("epv"), &mut vacc);
    add(g::GnssLocation::HAS_SPEED_ACCURACY, f("eps"), &mut sacc);
    add(g::GnssLocation::HAS_BEARING_ACCURACY, f("epd"), &mut bacc);
    loc.altitudeMeters = alt;
    loc.speedMetersPerSec = spd;
    loc.bearingDegrees = brg;
    loc.horizontalAccuracyMeters = hacc;
    loc.verticalAccuracyMeters = vacc;
    loc.speedAccuracyMetersPerSecond = sacc;
    loc.bearingAccuracyDegrees = bacc;
    Some(loc)
}

fn worker(callback: Callback, session: Arc<AtomicU64>, me: u64) {
    let current = || session.load(Ordering::SeqCst) == me;
    let cb = || callback.lock().unwrap().clone();
    if let Some(c) = cb() {
        let _ = c.gnssStatusCb(GnssStatusValue::SESSION_BEGIN);
    }
    let (mut svs_sent, mut locs_sent) = (0u64, 0u64);
    while current() {
        // gpsd's own default. Numeric, not "localhost": only the first
        // resolved address is tried, and that could be ::1.
        let host = getprop("service.gpsd.host", "127.0.0.1");
        let port = getprop("service.gpsd.port", "2947");
        let addr = match format!("{host}:{port}")
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
        {
            Some(a) => a,
            None => {
                log!("bad gpsd address {host}:{port}");
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
            Err(e) => log!("session {me}: connect {addr} failed: {e}"),
            Ok(mut s) => {
                log!("session {me}: connected to gpsd at {addr}");
                let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
                let _ = s.write_all(b"?WATCH={\"enable\":true,\"json\":true};\n");
                let mut r = BufReader::new(s);
                let mut line = String::new();
                while current() {
                    line.clear();
                    match r.read_line(&mut line) {
                        Ok(0) => {
                            log!("session {me}: gpsd closed the connection");
                            break;
                        }
                        Ok(_) => {}
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue
                        }
                        Err(e) => {
                            log!("session {me}: read error: {e}");
                            break;
                        }
                    }
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    let Some(c) = cb() else { continue };
                    match v.get("class").and_then(|x| x.as_str()) {
                        Some("SKY") => {
                            if let Some(svs) = sky_to_svs(&v) {
                                if let Err(e) = c.gnssSvStatusCb(&svs) {
                                    log!("gnssSvStatusCb failed: {e:?}");
                                } else {
                                    svs_sent += 1;
                                    if svs_sent == 1 || svs_sent % 60 == 0 {
                                        log!("session {me}: sent satellite list #{svs_sent} ({} satellites)", svs.len());
                                    }
                                }
                            }
                        }
                        Some("TPV") => {
                            if let Some(loc) = tpv_to_location(&v) {
                                if let Err(e) = c.gnssLocationCb(&loc) {
                                    log!("gnssLocationCb failed: {e:?}");
                                } else {
                                    locs_sent += 1;
                                    if locs_sent == 1 || locs_sent % 60 == 0 {
                                        log!(
                                            "session {me}: sent location #{locs_sent} {:.5},{:.5}",
                                            loc.latitudeDegrees,
                                            loc.longitudeDegrees
                                        );
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // Pause before reconnecting, checking for stop() every 200 ms.
        for _ in 0..10 {
            if !current() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    if let Some(c) = cb() {
        let _ = c.gnssStatusCb(GnssStatusValue::SESSION_END);
    }
    log!("session {me} ended ({svs_sent} satellite lists, {locs_sent} locations)");
}

impl Interface for Gnss {}

impl IGnss for Gnss {
    fn setCallback(&self, callback: &Strong<dyn IGnssCallback>) -> BinderResult<()> {
        log!("setCallback");
        *self.callback.lock().unwrap() = Some(callback.clone());
        callback.gnssSetCapabilitiesCb(0)?;
        callback.gnssSetSystemInfoCb(&GnssSystemInfo {
            yearOfHw: 2026,
            name: "gpsd-binder".into(),
        })?;
        Ok(())
    }
    fn close(&self) -> BinderResult<()> {
        log!("close");
        self.session.fetch_add(1, Ordering::SeqCst);
        *self.callback.lock().unwrap() = None;
        Ok(())
    }
    fn start(&self) -> BinderResult<()> {
        let me = self.session.fetch_add(1, Ordering::SeqCst) + 1;
        log!("start (session {me})");
        let (cb, s) = (self.callback.clone(), self.session.clone());
        std::thread::spawn(move || worker(cb, s, me));
        Ok(())
    }
    fn stop(&self) -> BinderResult<()> {
        log!("stop");
        self.session.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn setPositionMode(&self, o: &PositionModeOptions) -> BinderResult<()> {
        log!("setPositionMode minIntervalMs={}", o.minIntervalMs);
        Ok(())
    }
    fn injectTime(&self, _: i64, _: i64, _: i32) -> BinderResult<()> {
        Ok(())
    }
    fn injectLocation(&self, _: &GnssLocation) -> BinderResult<()> {
        Ok(())
    }
    fn injectBestLocation(&self, _: &GnssLocation) -> BinderResult<()> {
        Ok(())
    }
    fn deleteAidingData(&self, _: GnssAidingData) -> BinderResult<()> {
        Ok(())
    }
    fn startSvStatus(&self) -> BinderResult<()> {
        log!("startSvStatus");
        Ok(())
    }
    fn stopSvStatus(&self) -> BinderResult<()> {
        log!("stopSvStatus");
        Ok(())
    }
    fn startNmea(&self) -> BinderResult<()> {
        Ok(())
    }
    fn stopNmea(&self) -> BinderResult<()> {
        Ok(())
    }

    // Optional extensions: none. Answering "unsupported" (not "success with
    // no object") makes Android record each extension as absent.
    fn getExtensionPsds(&self) -> BinderResult<Option<Strong<dyn g::IGnssPsds::IGnssPsds>>> {
        unsupported()
    }
    fn getExtensionGnssConfiguration(
        &self,
    ) -> BinderResult<Strong<dyn g::IGnssConfiguration::IGnssConfiguration>> {
        unsupported()
    }
    fn getExtensionGnssMeasurement(
        &self,
    ) -> BinderResult<Strong<dyn g::IGnssMeasurementInterface::IGnssMeasurementInterface>> {
        unsupported()
    }
    fn getExtensionGnssPowerIndication(
        &self,
    ) -> BinderResult<Strong<dyn g::IGnssPowerIndication::IGnssPowerIndication>> {
        unsupported()
    }
    fn getExtensionGnssBatching(
        &self,
    ) -> BinderResult<Option<Strong<dyn g::IGnssBatching::IGnssBatching>>> {
        unsupported()
    }
    fn getExtensionGnssGeofence(
        &self,
    ) -> BinderResult<Option<Strong<dyn g::IGnssGeofence::IGnssGeofence>>> {
        unsupported()
    }
    fn getExtensionGnssNavigationMessage(
        &self,
    ) -> BinderResult<
        Option<Strong<dyn g::IGnssNavigationMessageInterface::IGnssNavigationMessageInterface>>,
    > {
        unsupported()
    }
    fn getExtensionAGnss(&self) -> BinderResult<Strong<dyn g::IAGnss::IAGnss>> {
        unsupported()
    }
    fn getExtensionAGnssRil(&self) -> BinderResult<Strong<dyn g::IAGnssRil::IAGnssRil>> {
        unsupported()
    }
    fn getExtensionGnssDebug(&self) -> BinderResult<Strong<dyn g::IGnssDebug::IGnssDebug>> {
        unsupported()
    }
    fn getExtensionGnssVisibilityControl(
        &self,
    ) -> BinderResult<
        Strong<dyn g::visibility_control::IGnssVisibilityControl::IGnssVisibilityControl>,
    > {
        unsupported()
    }
    fn getExtensionGnssAntennaInfo(
        &self,
    ) -> BinderResult<Strong<dyn g::IGnssAntennaInfo::IGnssAntennaInfo>> {
        unsupported()
    }
    fn getExtensionMeasurementCorrections(&self) -> BinderResult<Option<Strong<dyn g::measurement_corrections::IMeasurementCorrectionsInterface::IMeasurementCorrectionsInterface>>>{
        unsupported()
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    log!("starting; registering {SERVICE_NAME}");
    let service = BnGnss::new_binder(Gnss {
        callback: Arc::new(Mutex::new(None)),
        session: Arc::new(AtomicU64::new(0)),
    });
    rsbinder::serve(Uri::kernel())?
        .add(SERVICE_NAME, &service)?
        .run()?;
    Ok(())
}
