//! Transport-independent Android GNSS driver backed by gpsd.
//!
//! The service implements AIDL `android.hardware.gnss` version 2. `hal` implements the Android
//! callbacks and sessions; `gpsd` handles WATCH, JSON and NMEA. The service binary supplies an
//! owned transport for each connection and controls its lifetime through [`Stream`].

mod gpsd;
mod hal;

use std::{io, time::Duration};

use android_logger::{Config, FilterBuilder};
pub use gpsd::Stream;
use log::LevelFilter;
use rustix::time::{clock_gettime, ClockId};

/// Logs to logcat under `tag`: both programs' and this library's records from Info up, other
/// crates' (rsbinder's) from Warn up.
pub fn init_logging(tag: &str) {
    android_logger::init_once(
        Config::default()
            .with_tag(tag)
            .with_max_level(LevelFilter::Info)
            .format(|f, record| write!(f, "{}", record.args()))
            .with_filter(
                FilterBuilder::new()
                    .filter_level(LevelFilter::Warn)
                    .filter_module("gpsd_binder", LevelFilter::Info)
                    .filter_module("gpsd_binder_relay", LevelFilter::Info)
                    .build(),
            ),
    );
}

/// Time since boot, including time spent suspended: Android's `ElapsedRealtime` is
/// `CLOCK_BOOTTIME`.
fn boottime() -> Duration {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const CLOCK: ClockId = ClockId::Boottime;
    // Other hosts only compile this for the unit tests.
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const CLOCK: ClockId = ClockId::Monotonic;
    let ts = clock_gettime(CLOCK);
    Duration::new(
        u64::try_from(ts.tv_sec).expect("time since boot is not negative"),
        u32::try_from(ts.tv_nsec).expect("nanoseconds are below one second"),
    )
}

/// Registers `android.hardware.gnss.IGnss/default` and serves it until the process ends.
///
/// Each connection attempt calls `open`, which should return a [`Stream`] that is connected to
/// `gpsd`.  In production use on Android this is actually relayed through a simply relay process
/// over a UDS due to Android SELinux restructions, but at the level of this code this is just an
/// opaque bidirectional stream of bytes.
pub fn run_service(open: fn() -> io::Result<Box<dyn Stream>>) -> rsbinder::Result<()> {
    log::info!("starting; registering {}", hal::SERVICE_NAME);
    let service = hal::new_service(open, boottime);
    rsbinder::serve(rsbinder::Uri::kernel())
        .and_then(|server| server.add(hal::SERVICE_NAME, &service))
        .and_then(|server| server.run())
}
