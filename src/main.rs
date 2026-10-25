//! gpsd-binder: an `android.hardware.gnss` AIDL (version 2) driver written in pure Rust with
//! rsbinder, built outside the Android tree.
//!
//! STATUS: prototype. The approach was proved on Waydroid (LineageOS 20, Android 13, x86_64) on
//! 2026-10-24; see transfer.md. It is not yet a finished driver.
//!
//! The driver registers `android.hardware.gnss.IGnss/default` with servicemanager. When Android
//! starts a GNSS session it connects to gpsd (`service.gpsd.host` and `service.gpsd.port`, default
//! 127.0.0.1:2947) and passes gpsd's satellite lists (SKY) and fixes (TPV) to Android through
//! `IGnssCallback`.
//!
//! [`hal`] is the Android side: the `IGnss` service, its sessions and the mapping from gpsd's
//! reports to the AIDL types. [`gpsd`] is the client for gpsd's JSON protocol and knows nothing
//! about Android. This file holds what is specific to the Android system the driver runs on: system
//! properties, the boot-time clock, and logging to stderr, which the init file sends to the kernel
//! log (`stdio_to_kmsg`).

mod gpsd;
mod hal;

use std::{
    io::{self, Write},
    net::{SocketAddr, ToSocketAddrs},
    process::Command,
    time::Duration,
};

use rustix::time::{clock_gettime, ClockId};

/// Writes each record to stderr with one `write()`: init's `stdio_to_kmsg` turns every write into a
/// separate kernel log record.
struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            let line = format!("gpsd-binder: {}\n", record.args());
            let _ = io::stderr().lock().write_all(line.as_bytes());
        }
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

fn getprop(name: &str, default: &str) -> String {
    Command::new("/system/bin/getprop")
        .arg(name)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

/// gpsd's address from the system properties. The feed asks before every connection attempt, so a
/// changed property takes effect at the next one.
fn gpsd_address() -> Option<SocketAddr> {
    // gpsd's own default. Numeric, not "localhost": only the first resolved address is used, and that
    // could be ::1.
    let host = getprop("service.gpsd.host", "127.0.0.1");
    let port = getprop("service.gpsd.port", "2947");
    let addr = format!("{host}:{port}")
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next());
    if addr.is_none() {
        log::warn!("bad gpsd address {host}:{port}");
    }
    addr
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    log::set_logger(&LOGGER)?;
    log::set_max_level(log::LevelFilter::Info);
    log::info!("starting; registering {}", hal::SERVICE_NAME);
    let service = hal::new_service(gpsd_address, boottime);
    rsbinder::serve(rsbinder::Uri::kernel())?
        .add(hal::SERVICE_NAME, &service)?
        .run()?;
    Ok(())
}
