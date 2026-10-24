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
  that implements only the AIDL interface. Only Android 13 has been tested.
- **SELinux disabled or permissive.** There is no SELinux policy for the
  driver yet. Waydroid runs Android with SELinux disabled.
- **x86_64 or arm64.** Only x86_64 has been tested.

The program needs only `libc.so` and `libdl.so`, so it does not depend on
the libraries of one Android release.

## Build

```
rustup target add x86_64-linux-android
ANDROID_NDK_HOME=/path/to/android-ndk-r27c scripts/build.sh x86_64 30
```

## Install

| File                                                    | Installed as                                               |
| ------------------------------------------------------- | ---------------------------------------------------------- |
| `target/x86_64-linux-android/release/gpsd-binder`       | `/vendor/bin/hw/android.hardware.gnss-service.gpsd-binder` |
| `android/android.hardware.gnss-service.gpsd-binder.rc`  | `/vendor/etc/init/`                                        |
| `android/android.hardware.gnss-service.gpsd-binder.xml` | `/vendor/etc/vintf/manifest/`                              |

Android looks for a GNSS driver only once, at boot, so restart Android after
installing.

## Configuration

| Property            | Default     | Meaning                                            |
| ------------------- | ----------- | -------------------------------------------------- |
| `service.gpsd.host` | `127.0.0.1` | gpsd's IP address or host name, as Android sees it |
| `service.gpsd.port` | `2947`      | gpsd's TCP port                                    |

The driver reads both properties each time Android starts a GNSS session.
To change them, add an init file of your own that sets them, for example in
an `on early-boot` block.

Any process that can open a TCP connection to gpsd can read positions
without Android's location permission, and can send commands to gpsd. Where
that matters, restrict which processes can connect.
`waydroid/gpsd-access-guard.sh` does this with an iptables rule.

## Integrations

- [`waydroid/`](waydroid/README.md): gpsd on a Linux host, Android in
  Waydroid.
