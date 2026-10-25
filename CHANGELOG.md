# Changelog

All notable changes to this project are recorded in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- NMEA sentences. The driver asks gpsd for NMEA as well as JSON, and passes
  each sentence to Android unchanged while an app listens for NMEA.

### Changed

- The driver reports the scheduling capability, so Android keeps it running
  between fixes instead of stopping and restarting it for apps that ask for
  a location less often than every 10 seconds.

### Fixed

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
