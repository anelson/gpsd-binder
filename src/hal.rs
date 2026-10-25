//! The android.hardware.gnss service: the `IGnss` implementation, the sessions it runs, and the
//! mapping from gpsd's reports to the AIDL types.
//!
//! Each `start()` begins a new numbered session. A thread runs the gpsd feed while its number is
//! current, so `stop()` and a later `start()` never overlap. gpsd's NMEA sentences are passed on
//! unchanged between `startNmea()` and `stopNmea()`. Every optional extension answers "unsupported
//! operation", which Android records as absent.

use std::{
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, PoisonError,
    },
    thread,
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};
use rsbinder::*;

use crate::gpsd::{self, Event, GnssId, Report, Satellite, SatelliteId, Tpv};

#[expect(
    unreachable_pub,
    clippy::allow_attributes,
    clippy::unused_self,
    reason = "rsbinder-aidl generated code; it carries its own allow attributes for the rest"
)]
mod gen {
    include!(concat!(env!("OUT_DIR"), "/gnss_aidl.rs"));
}

use g::{
    measurement_corrections::IMeasurementCorrectionsInterface::IMeasurementCorrectionsInterface,
    visibility_control::IGnssVisibilityControl::IGnssVisibilityControl,
    ElapsedRealtime::ElapsedRealtime,
    GnssConstellationType::GnssConstellationType as Constellation,
    GnssLocation::GnssLocation,
    IGnss::{BnGnss, GnssAidingData::GnssAidingData, IGnss, PositionModeOptions::PositionModeOptions},
    IGnssCallback::{
        GnssStatusValue::GnssStatusValue, GnssSvFlags::GnssSvFlags, GnssSvInfo::GnssSvInfo,
        GnssSystemInfo::GnssSystemInfo, IGnssCallback,
    },
};
use gen::android::hardware::gnss as g;

pub(crate) const SERVICE_NAME: &str = "android.hardware.gnss.IGnss/default";

/// Opens a new stream to gpsd, for each connection attempt.
pub(crate) type OpenFn = fn() -> io::Result<Box<dyn gpsd::Stream>>;

/// Time since boot, including time spent suspended: Android's `ElapsedRealtime`. It also drives the
/// feed's stale-gap check.
pub(crate) type ClockFn = fn() -> Duration;

/// The framework's callback object, shared by the service and its session threads.
#[derive(Clone, Default)]
struct Callback(Arc<Mutex<Option<Strong<dyn IGnssCallback>>>>);

impl Callback {
    fn set(&self, callback: Option<Strong<dyn IGnssCallback>>) {
        *self.lock() = callback;
    }

    /// A clone, so that no lock is held during a binder call.
    fn get(&self) -> Option<Strong<dyn IGnssCallback>> {
        self.lock().clone()
    }

    fn lock(&self) -> MutexGuard<'_, Option<Strong<dyn IGnssCallback>>> {
        // Nothing panics while holding this lock, so it is never poisoned; take the value anyway.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Gnss {
    callback: Callback,
    /// Bumped by every `start()`, `stop()` and `close()`. A session thread runs only while its
    /// number is current, so `stop()` and a later `start()` never overlap.
    session: Arc<AtomicU64>,
    /// Set by `startNmea()` and cleared by `stopNmea()`. Android calls them when the first app
    /// starts to listen for NMEA and when the last one stops, with or without a session, so the
    /// setting holds across sessions.
    nmea: Arc<AtomicBool>,
    open: OpenFn,
    clock: ClockFn,
}

/// The service object to register as [`SERVICE_NAME`].
pub(crate) fn new_service(open: OpenFn, clock: ClockFn) -> Strong<dyn IGnss> {
    BnGnss::new_binder(Gnss {
        callback: Callback::default(),
        session: Arc::new(AtomicU64::new(0)),
        nmea: Arc::new(AtomicBool::new(false)),
        open,
        clock,
    })
}

fn unsupported<T>() -> BinderResult<T> {
    Err(ExceptionCode::UnsupportedOperation.into())
}

impl From<GnssId> for Constellation {
    fn from(id: GnssId) -> Constellation {
        match id {
            GnssId::Gps => Constellation::GPS,
            GnssId::Sbas => Constellation::SBAS,
            GnssId::Galileo => Constellation::GALILEO,
            GnssId::BeiDou => Constellation::BEIDOU,
            GnssId::Qzss => Constellation::QZSS,
            GnssId::Glonass => Constellation::GLONASS,
            GnssId::NavIc => Constellation::IRNSS,
            // Android has no value for IMES.
            GnssId::Imes | GnssId::Other => Constellation::UNKNOWN,
        }
    }
}

impl GnssSvInfo {
    /// A satellite as Android expects it, or None when gpsd's numbers do not identify it.
    fn from_satellite(s: &Satellite) -> Option<GnssSvInfo> {
        let (constellation, svid) = match s.id()? {
            // Android numbers QZSS by PRN.
            SatelliteId::Svid(GnssId::Qzss, n) => (Constellation::QZSS, n + 192),
            // Android has no IMES; 173-182 are its PRNs.
            SatelliteId::Svid(GnssId::Imes, n) => (Constellation::UNKNOWN, n + 172),
            SatelliteId::Svid(gnssid, n) => (Constellation::from(gnssid), n),
            SatelliteId::GlonassChannel(channel) => (Constellation::GLONASS, channel + 100),
        };
        Some(GnssSvInfo {
            svid,
            constellation,
            cN0Dbhz: s.ss.unwrap_or(0.0) as f32,
            basebandCN0DbHz: 0.0,
            elevationDegrees: s.el.unwrap_or(0.0) as f32,
            azimuthDegrees: s.az.unwrap_or(0.0) as f32,
            carrierFrequencyHz: 0,
            svFlag: if s.used {
                GnssSvFlags::USED_IN_FIX.0
            } else {
                GnssSvFlags::NONE.0
            },
        })
    }
}

impl GnssLocation {
    /// A fix as Android expects it, or None when the report is not a fix with a position, a time
    /// and a horizontal accuracy. Android requires the accuracy: on a location without one, Android
    /// 13's `system_server` throws an exception and the whole framework restarts. `elapsed` is
    /// the boot-time clock when the report arrived.
    fn from_tpv(tpv: &Tpv, elapsed: Duration) -> Option<GnssLocation> {
        if !tpv.mode.has_fix() {
            return None;
        }
        let mut flags = g::GnssLocation::HAS_LAT_LONG | g::GnssLocation::HAS_HORIZONTAL_ACCURACY;
        // A field is flagged only when gpsd gave a value; Android reads it only then.
        let mut field = |flag: i32, value: Option<f64>| {
            value.map_or(0.0, |v| {
                flags |= flag;
                v
            })
        };
        let altitude = field(g::GnssLocation::HAS_ALTITUDE, tpv.alt_hae);
        let speed = field(g::GnssLocation::HAS_SPEED, tpv.speed);
        let bearing = field(g::GnssLocation::HAS_BEARING, tpv.track);
        let vertical_accuracy = field(g::GnssLocation::HAS_VERTICAL_ACCURACY, tpv.epv);
        let speed_accuracy = field(g::GnssLocation::HAS_SPEED_ACCURACY, tpv.eps);
        let bearing_accuracy = field(g::GnssLocation::HAS_BEARING_ACCURACY, tpv.epd);
        Some(GnssLocation {
            gnssLocationFlags: flags,
            latitudeDegrees: tpv.lat?,
            longitudeDegrees: tpv.lon?,
            altitudeMeters: altitude,
            speedMetersPerSec: speed,
            bearingDegrees: bearing,
            horizontalAccuracyMeters: tpv.eph?,
            verticalAccuracyMeters: vertical_accuracy,
            speedAccuracyMetersPerSecond: speed_accuracy,
            bearingAccuracyDegrees: bearing_accuracy,
            timestampMillis: tpv.time?.timestamp_millis(),
            elapsedRealtime: ElapsedRealtime {
                flags: g::ElapsedRealtime::HAS_TIMESTAMP_NS,
                timestampNs: i64::try_from(elapsed.as_nanos())
                    .expect("time since boot fits in 63 bits of nanoseconds for 292 years"),
                timeUncertaintyNs: 0.0,
            },
        })
    }
}

/// One GNSS session: the gpsd feed, and what Android gets from it.
struct Session {
    number: u64,
    current_session: Arc<AtomicU64>,
    nmea: Arc<AtomicBool>,
    callback: Callback,
    clock: ClockFn,
}

impl Session {
    fn current(&self) -> bool {
        self.current_session.load(Ordering::SeqCst) == self.number
    }

    fn run(self, open: OpenFn) {
        let me = self.number;
        let stop = {
            let session = Arc::clone(&self.current_session);
            move || session.load(Ordering::SeqCst) != me
        };
        let feed = gpsd::Feed::new(
            open,
            gpsd::Watch::default(),
            gpsd::Config::default(),
            stop,
            self.clock,
        );
        let (mut svs_sent, mut locs_sent, mut nmea_sent) = (0u64, 0u64, 0u64);
        for event in feed {
            // stop() may have been called while the feed waited for this event.
            if !self.current() {
                break;
            }
            match event {
                Event::Connected(version) => {
                    log::info!("session {me}: connected to gpsd {}", version.release);
                }
                Event::ConnectFailed(error) => log::warn!("session {me}: connect failed: {error}"),
                Event::Disconnected(reason) => log::info!("session {me}: {reason}"),
                Event::Report(Report::Devices(devices)) => {
                    for device in &devices.devices {
                        log::info!("session {me}: gpsd has {device}");
                    }
                }
                Event::Report(Report::Sky(sky)) => {
                    if let Some(sats) = &sky.satellites {
                        let svs: Vec<GnssSvInfo> =
                            sats.iter().filter_map(GnssSvInfo::from_satellite).collect();
                        if self.send("gnssSvStatusCb", |c| c.gnssSvStatusCb(&svs)) {
                            svs_sent += 1;
                            if svs_sent == 1 || svs_sent % 60 == 0 {
                                log::info!(
                                    "session {me}: sent satellite list #{svs_sent} ({} satellites)",
                                    svs.len()
                                );
                            }
                        }
                    }
                }
                Event::Report(Report::Tpv(tpv)) => {
                    if let Some(loc) = GnssLocation::from_tpv(&tpv, (self.clock)()) {
                        if self.send("gnssLocationCb", |c| c.gnssLocationCb(&loc)) {
                            locs_sent += 1;
                            if locs_sent == 1 || locs_sent % 60 == 0 {
                                log::info!(
                                    "session {me}: sent location #{locs_sent} {:.5},{:.5}",
                                    loc.latitudeDegrees,
                                    loc.longitudeDegrees
                                );
                            }
                        }
                    }
                }
                // Only between startNmea() and stopNmea(): Android 13 checks for listeners only for a
                // version 1 driver.
                Event::Nmea(sentence) => {
                    if self.nmea.load(Ordering::SeqCst) {
                        let timestamp = DateTime::<Utc>::from(SystemTime::now()).timestamp_millis();
                        if self.send("gnssNmeaCb", |c| c.gnssNmeaCb(timestamp, &sentence)) {
                            nmea_sent += 1;
                            if nmea_sent == 1 {
                                log::info!("session {me}: sent NMEA sentence #1 {}", sentence.trim_end());
                            }
                        }
                    }
                }
                Event::Report(Report::Error(e)) => log::warn!("session {me}: gpsd: {}", e.message),
                Event::Report(
                    Report::Version(_) | Report::Device(_) | Report::Watch(_) | Report::Unknown,
                ) => {}
            }
        }
        log::info!(
            "session {me} ended ({svs_sent} satellite lists, {locs_sent} locations, {nmea_sent} NMEA \
             sentences)"
        );
    }

    /// One call on the callback. False if there is no callback or the call failed.
    fn send(&self, name: &str, call: impl FnOnce(&Strong<dyn IGnssCallback>) -> BinderResult<()>) -> bool {
        let Some(callback) = self.callback.get() else {
            return false;
        };
        match call(&callback) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("{name} failed: {e:?}");
                false
            }
        }
    }
}

impl Gnss {
    /// Session status is sent from `start()` and `stop()`, not from the session thread, so that it
    /// reaches Android in the order of those calls. A thread that has not yet noticed a `stop()`
    /// could otherwise send "session end" after the next `start()` had sent "session begin".
    fn send_status(&self, status: GnssStatusValue) {
        if let Some(c) = self.callback.get() {
            if let Err(e) = c.gnssStatusCb(status) {
                log::warn!("gnssStatusCb failed: {e:?}");
            }
        }
    }
}

impl Interface for Gnss {}

impl IGnss for Gnss {
    fn setCallback(&self, callback: &Strong<dyn IGnssCallback>) -> BinderResult<()> {
        log::info!("setCallback");
        self.callback.set(Some(callback.clone()));
        // Set the SCHEDULING capability for this driver.
        //
        // SCHEDULING says that the driver handles the request interval itself. Without it, if the
        // requested update interval is over 10s, Android stops the driver after each fix and starts it
        // again by alarm. For this specific driver that means we would drop the connection to
        // `gpsd`, only to have to connect to it again.  Presumably this feature exists to save
        // power by powering down the receiver between fixes, but with `gpsd` that really isn't
        // ideal, and doing so is unlikely to save much power anyway.
        //
        // Despite claiming this capability, this driver reports every fix as gpsd reports it,
        // whatever the interval: Android drops deliveries that come sooner than each app asked for,
        // and gpsd keeps the receiver running whether or not Android takes the fixes, so spacing or
        // postponing them would save no power.
        if let Err(e) = callback.gnssSetCapabilitiesCb(g::IGnssCallback::CAPABILITY_SCHEDULING) {
            log::warn!("gnssSetCapabilitiesCb failed: {e:?}");
        }
        if let Err(e) = callback.gnssSetSystemInfoCb(&GnssSystemInfo {
            yearOfHw: 2026,
            name: "gpsd-binder".into(),
        }) {
            log::warn!("gnssSetSystemInfoCb failed: {e:?}");
        }
        Ok(())
    }

    fn close(&self) -> BinderResult<()> {
        log::info!("close");
        self.session.fetch_add(1, Ordering::SeqCst);
        self.callback.set(None);
        Ok(())
    }

    fn start(&self) -> BinderResult<()> {
        let number = self.session.fetch_add(1, Ordering::SeqCst) + 1;
        log::info!("start (session {number})");
        self.send_status(GnssStatusValue::SESSION_BEGIN);
        let session = Session {
            number,
            current_session: Arc::clone(&self.session),
            nmea: Arc::clone(&self.nmea),
            callback: self.callback.clone(),
            clock: self.clock,
        };
        let open = self.open;
        thread::spawn(move || session.run(open));
        Ok(())
    }

    fn stop(&self) -> BinderResult<()> {
        log::info!("stop");
        self.session.fetch_add(1, Ordering::SeqCst);
        self.send_status(GnssStatusValue::SESSION_END);
        Ok(())
    }

    fn setPositionMode(&self, o: &PositionModeOptions) -> BinderResult<()> {
        // The interval is not used; see SCHEDULING in `setCallback()` for the reasoning here.
        log::info!("setPositionMode minIntervalMs={}", o.minIntervalMs);
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
        log::info!("startSvStatus");
        Ok(())
    }

    fn stopSvStatus(&self) -> BinderResult<()> {
        log::info!("stopSvStatus");
        Ok(())
    }

    fn startNmea(&self) -> BinderResult<()> {
        log::info!("startNmea");
        self.nmea.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn stopNmea(&self) -> BinderResult<()> {
        log::info!("stopNmea");
        self.nmea.store(false, Ordering::SeqCst);
        Ok(())
    }

    // Optional extensions: none. Answering "unsupported" (not "success with no object") makes Android
    // record each extension as absent.
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

    fn getExtensionGnssBatching(&self) -> BinderResult<Option<Strong<dyn g::IGnssBatching::IGnssBatching>>> {
        unsupported()
    }

    fn getExtensionGnssGeofence(&self) -> BinderResult<Option<Strong<dyn g::IGnssGeofence::IGnssGeofence>>> {
        unsupported()
    }

    fn getExtensionGnssNavigationMessage(
        &self,
    ) -> BinderResult<Option<Strong<dyn g::IGnssNavigationMessageInterface::IGnssNavigationMessageInterface>>>
    {
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

    fn getExtensionGnssVisibilityControl(&self) -> BinderResult<Strong<dyn IGnssVisibilityControl>> {
        unsupported()
    }

    fn getExtensionGnssAntennaInfo(&self) -> BinderResult<Strong<dyn g::IGnssAntennaInfo::IGnssAntennaInfo>> {
        unsupported()
    }

    fn getExtensionMeasurementCorrections(
        &self,
    ) -> BinderResult<Option<Strong<dyn IMeasurementCorrectionsInterface>>> {
        unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpsd::protocol::tests::samples;

    fn tpv(line: &str) -> Tpv {
        match serde_json::from_str(line).unwrap() {
            Report::Tpv(t) => t,
            other => panic!("not a TPV: {other:?}"),
        }
    }

    fn satellites(line: &str) -> Vec<GnssSvInfo> {
        match serde_json::from_str(line).unwrap() {
            Report::Sky(s) => s
                .satellites
                .unwrap()
                .iter()
                .filter_map(GnssSvInfo::from_satellite)
                .collect(),
            other => panic!("not a SKY: {other:?}"),
        }
    }

    const ELAPSED: Duration = Duration::from_nanos(123_456_789_012);

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the values are parsed from literals in the sample"
    )]
    fn a_full_fix_maps_every_value_and_flags_each_one() {
        let loc = GnssLocation::from_tpv(&tpv(samples::TPV_3D), ELAPSED).unwrap();
        assert_eq!(
            loc.gnssLocationFlags,
            g::GnssLocation::HAS_LAT_LONG
                | g::GnssLocation::HAS_ALTITUDE
                | g::GnssLocation::HAS_SPEED
                | g::GnssLocation::HAS_BEARING
                | g::GnssLocation::HAS_HORIZONTAL_ACCURACY
                | g::GnssLocation::HAS_VERTICAL_ACCURACY
                | g::GnssLocation::HAS_SPEED_ACCURACY
                | g::GnssLocation::HAS_BEARING_ACCURACY
        );
        assert_eq!(
            (loc.latitudeDegrees, loc.longitudeDegrees),
            (44.069009667, -121.314245833)
        );
        // altHAE, not altMSL (1147.0) or the deprecated alt.
        assert_eq!(loc.altitudeMeters, 1125.7);
        assert_eq!((loc.speedMetersPerSec, loc.bearingDegrees), (0.043, 12.5));
        assert_eq!(
            (loc.horizontalAccuracyMeters, loc.verticalAccuracyMeters),
            (17.29, 29.21)
        );
        assert_eq!(
            (loc.speedAccuracyMetersPerSecond, loc.bearingAccuracyDegrees),
            (21.89, 1.5)
        );
        assert_eq!(loc.timestampMillis, 2_046_014_367_000);
        assert_eq!(loc.elapsedRealtime.flags, g::ElapsedRealtime::HAS_TIMESTAMP_NS);
        assert_eq!(loc.elapsedRealtime.timestampNs, 123_456_789_012);
    }

    #[test]
    fn a_bare_fix_flags_only_the_position_and_its_accuracy() {
        let line =
            r#"{"class":"TPV","mode":2,"time":"2026-10-24T20:48:31.250Z","lat":1.5,"lon":-2.5,"eph":9.5}"#;
        let loc = GnssLocation::from_tpv(&tpv(line), ELAPSED).unwrap();
        assert_eq!(
            loc.gnssLocationFlags,
            g::GnssLocation::HAS_LAT_LONG | g::GnssLocation::HAS_HORIZONTAL_ACCURACY
        );
        assert_eq!((loc.latitudeDegrees, loc.longitudeDegrees), (1.5, -2.5));
        // Fractional seconds are kept.
        assert_eq!(loc.timestampMillis, 1_792_874_911_250);
        assert_eq!((loc.altitudeMeters, loc.horizontalAccuracyMeters), (0.0, 9.5));
    }

    #[test]
    fn reports_without_a_timed_position_and_accuracy_are_dropped() {
        for line in [
            samples::TPV_NO_FIX,
            // Mode 2 from a GLL sentence: a position but no date.
            r#"{"class":"TPV","mode":2,"lat":1.0,"lon":2.0,"eph":9.5}"#,
            r#"{"class":"TPV","mode":3,"time":"2026-10-24T20:48:31.250Z","lat":1.0,"eph":9.5}"#,
            // No accuracy: gpsd sends this from an RMC sentence that comes before the GGA and GSA
            // sentences of the same cycle.
            r#"{"class":"TPV","mode":3,"time":"2034-11-01T17:19:25.000Z","lat":44.069,"lon":-121.314}"#,
            // An unknown mode is not a fix.
            samples::TPV_ODD,
        ] {
            assert!(GnssLocation::from_tpv(&tpv(line), ELAPSED).is_none(), "{line}");
        }
    }

    #[test]
    fn satellites_are_numbered_by_svid_within_their_constellation() {
        let svs = satellites(samples::SKY);
        // The fourth satellite's constellation is unknown, so it is left out.
        assert_eq!(svs.len(), 3);
        let gps = &svs[0];
        assert_eq!((gps.svid, gps.constellation), (1, Constellation::GPS));
        assert_eq!(
            (gps.elevationDegrees, gps.azimuthDegrees, gps.cN0Dbhz),
            (53.0, 280.0, 36.0)
        );
        assert_eq!(gps.svFlag, GnssSvFlags::USED_IN_FIX.0);
        // GLONASS slot 3, not gpsd's PRN 67.
        let glonass = &svs[1];
        assert_eq!((glonass.svid, glonass.constellation), (3, Constellation::GLONASS));
        assert_eq!(svs[2].constellation, Constellation::GALILEO);
        // Values gpsd leaves out are sent as 0.
        let bare = satellites(r#"{"class":"SKY","satellites":[{"PRN":7}]}"#);
        assert_eq!(bare[0].svFlag, GnssSvFlags::NONE.0);
        assert_eq!((bare[0].elevationDegrees, bare[0].cN0Dbhz), (0.0, 0.0));
    }

    #[test]
    fn android_numbers() {
        for (sat, constellation, svid) in [
            (r#"{"PRN":193,"gnssid":5,"svid":1}"#, Constellation::QZSS, 193),
            (r#"{"PRN":173,"gnssid":4,"svid":1}"#, Constellation::UNKNOWN, 173),
            // A GLONASS satellite of unknown slot, by its frequency channel, -7 and 6.
            (
                r#"{"PRN":-1,"gnssid":6,"svid":255,"freqid":0}"#,
                Constellation::GLONASS,
                93,
            ),
            (
                r#"{"PRN":-1,"gnssid":6,"svid":255,"freqid":13}"#,
                Constellation::GLONASS,
                106,
            ),
            // Outside Android's documented range, but the satellite's own number.
            (r#"{"PRN":158,"gnssid":1,"svid":158}"#, Constellation::SBAS, 158),
        ] {
            let sv = GnssSvInfo::from_satellite(&serde_json::from_str(sat).unwrap()).unwrap();
            assert_eq!((sv.constellation, sv.svid), (constellation, svid), "{sat}");
        }
    }
}
