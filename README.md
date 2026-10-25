# gpsd-binder

An Android GNSS driver, written in Rust, that takes its data from
[gpsd](https://gpsd.io/). It implements Android's `android.hardware.gnss`
AIDL interface (version 2, Android 13) over binder with
[rsbinder](https://github.com/hiking90/rsbinder), and is built with `cargo`
and the Android NDK, outside the Android source tree.

Android apps then get real positions and satellites from whatever receiver
gpsd owns, through Android's own `gps` location provider, with no mock
location. gpsd keeps the receiver, so Linux programs keep working too.

The driver connects to gpsd over TCP. gpsd can run on the same system as
Android, or on any host that Android can reach, such as the Linux host of a
container. The driver does not depend on where it runs. Files that adapt it
to one environment are kept separate from it (see [Integrations](#integrations)).

## Requirements

- **Android 13 or later.** Android 12 and earlier cannot use a GNSS driver
  that implements only the AIDL interface. Android 13, 16 and 17 have been
  tested in arm64 AOSP emulator images with SELinux enforcing.
- **Root access to install vendor files and SELinux policy.** Policy is
  supplied in `android/sepolicy/`. Android 17 also needs the supplied
  system permission file for network access. Waydroid with Android 13 has
  SELinux disabled and needs neither.
- **x86_64 or arm64.** x86_64 was tested on Waydroid with the earlier
  single-program build; the current two-program build was tested on arm64
  in the emulator. Physical arm64 devices have not been tested.

The GNSS service starts a separate relay process for TCP: Android's SELinux
rules forbid HAL servers from using TCP sockets. Both programs need only
`libc.so`, `libdl.so` and `liblog.so`, so they do not depend on the libraries
of one Android release.

## Build

```
ANDROID_NDK_HOME=/path/to/android-ndk-r30 just build x86_64
```

[BUILDING.md](BUILDING.md) covers the toolchain, the NDK, the checks and
the tests.

## Install

Releases carry a ready-made build for x86_64 and arm64 Android.
[INSTALLING.md](INSTALLING.md) describes how to install and configure it, on
the Android emulator, Waydroid on Linux, and other Android devices.

## Integrations

- [`waydroid/`](waydroid/README.md): gpsd on a Linux host, Android in
  Waydroid.

## Credits

gpsd-binder was inspired by @bzobl/Bernd Zobl's
[waydroid-gps](https://github.com/bzobl/waydroid-gps), which gives Android apps
in Waydroid real positions from gpsd through a GNSS driver. That driver, on the
[`waydroid-gps` branch of
bzobl/gpsd](https://github.com/bzobl/gpsd/tree/waydroid-gps), makes the HIDL
GNSS 1.1 driver that Adam Serbinski added to gpsd's
[`android/`](https://gitlab.com/gpsd/gpsd/-/tree/master/android) directory in
2019 work again. gpsd-binder is a separate implementation: an AIDL driver,
written in Rust, that builds without an Android source tree.
