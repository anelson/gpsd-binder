# Changelog

All notable changes to this project are recorded in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- NMEA sentences. The driver asks gpsd for NMEA as well as JSON, and passes
  each sentence to Android unchanged while an app listens for NMEA.

## [0.1.0] - 2026-10-25

### Added

- An Android GNSS driver that implements the AIDL `android.hardware.gnss`
  interface (version 2, Android 13 and later) and takes positions and
  satellites from gpsd over TCP.
- Release builds for x86_64 and arm64 Android. Only x86_64 has been tested on
  a device.
- Files that install the driver on Waydroid, with gpsd on the Linux host.
