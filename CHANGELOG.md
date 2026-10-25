# Changelog

All notable changes to this project are recorded in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- SELinux policy for enforcing Android images, with separate domains for
  the GNSS service and its TCP relay, plus a neverallow check script.
- Android emulator installation instructions, separate from the Waydroid
  guide, including policy installation, upgrades and removal.
- A system permission file for Android 17's native-uid network checks. It
  grants `INTERNET` and `ACCESS_LOCAL_NETWORK` to uid `gps`.
- NMEA sentences. The driver asks gpsd for NMEA as well as JSON, and passes
  each sentence to Android unchanged while an app listens for NMEA.

### Changed

- The service starts `gpsd-binder-relay` for each connection to gpsd.
  Android forbids HAL servers from using TCP sockets, so only the relay
  connects to the network. Both programs are required at installation.
- Both programs run as Android's `gps` user (1021), not root. The Waydroid
  access guard admits that uid instead of uid 0.
- Logs go to Android's logcat under `gpsd-binder` and `gpsd-binder-relay`,
  not to the kernel log. Both programs now also link against `liblog.so`.
- The driver reports the scheduling capability, so Android keeps it running
  between fixes instead of stopping and restarting it for apps that ask for
  a location less often than every 10 seconds.

### Fixed

- A fix without a horizontal accuracy no longer reaches Android. Android 13
  throws an exception in system_server on such a location, and the whole
  framework restarts. gpsd sends one when a receiver's RMC sentence comes
  before the GGA and GSA sentences of the same cycle.
- Satellite numbers. QZSS satellites have their PRNs (193 and up), as
  Android expects. Satellites from receivers whose gpsd driver gives one
  number over all constellations (GeoStar, NavCom, NMEA 2000, Oncore,
  SuperStar II and others) appear in their constellation instead of as
  unknown. SBAS satellites from Javad receivers and some from Trimble
  Copernicus receivers, GLONASS satellites from Allystar receivers and NavIC
  satellites from Skytraq receivers have their real numbers and
  constellations. A satellite that gpsd does not identify is left out.

## [0.1.0] - 2026-10-25

### Added

- An Android GNSS driver that implements the AIDL `android.hardware.gnss`
  interface (version 2, Android 13 and later) and takes positions and
  satellites from gpsd over TCP.
- Release builds for x86_64 and arm64 Android. Only x86_64 has been tested on
  a device.
- Files that install the driver on Waydroid, with gpsd on the Linux host.
