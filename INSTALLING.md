# Installing gpsd-binder

The installers download the right build for Android, check its checksum, and install both
programs and their configuration. No Rust or Android source tree is needed. Run the same command
again to update. They do not configure or restart gpsd.

You need gpsd 3.20 or later, either serving a receiver or replaying a recording with gpsfake.
Apps get positions through Android's normal `gps` provider, not mock location.

## Waydroid on Linux

Requires Waydroid with Android 13 (LineageOS 20), a Wayland desktop, sudo, and `socat`.
Install `socat` with your distribution's package manager if needed. Start Waydroid, then run
these commands **as your desktop user, not root**:

```sh
curl -fLO https://github.com/anelson/gpsd-binder/releases/latest/download/install-waydroid.sh
sh install-waydroid.sh
```

The script asks for sudo, detects the bridge address and Android architecture, installs the
files, and restarts your Waydroid session. By default it connects to gpsd on the Linux host's
loopback port 2947 through a bridge-only forwarder. An access guard restricts Android-side
connections to the driver's uid.

Enable Android Location, open ATAK or another location app, and grant precise location access.
An app must request location before the driver connects to gpsd.

## Android emulator

On Linux or macOS, install Android Studio or the SDK command-line tools. Create an **AOSP
(default)** AVD for Android 13, 16 or 17 (API 33, 36 or 37). Play Store images do not allow root
and cannot be used. Add the SDK's `platform-tools` and `emulator` directories to `PATH`.

With an AVD named `gpsd`:

```sh
curl -fLO https://github.com/anelson/gpsd-binder/releases/latest/download/install-emulator.sh
sh install-emulator.sh --avd gpsd
```

The script starts the AVD with writable partitions and snapshots disabled, installs and checks
the SELinux policy, reboots, installs the driver in place of the emulator's built-in GNSS HAL,
and reboots again. SELinux stays enforcing. Android 17's network permission file is installed
automatically. Allow a few minutes.

If the emulator is already running, omit `--avd`; with several running, select one with
`--serial emulator-5554`. An already-running AVD must have been started with
`-writable-system -no-snapshot`. Keep those flags on future starts too:

```sh
emulator -avd gpsd -writable-system -no-snapshot
```

Enable Android Location and open an app that requests precise location, such as
[GPSTest](https://github.com/barbeau/gpstest). Do not use the emulator's Location controls.
The default gpsd endpoint is `10.0.2.2:2947`, which reaches the host's loopback.

## Another gpsd address or port

Both scripts accept `--host` and `--port`, for example:

```sh
sh install-emulator.sh --avd gpsd --host 192.168.1.20 --port 2948
sh install-waydroid.sh --port 2948
```

For Waydroid, omit `--host` to use the Linux host's loopback. An explicit `--host` connects
Android directly to that address and must resolve to IPv4 for the access guard. Prefer numeric
addresses; DNS from the emulator relay's SELinux domain has not been tested.

Keep gpsd on a trusted network. Its TCP interface does not enforce Android location permissions
and can permit receiver reconfiguration. The emulator installer does not install Waydroid's
access guard; use trusted apps in the AVD.

## If it does not work

The scripts stop with an error for unrecognized images, existing unrelated GNSS configuration,
or an unavailable prerequisite. No current fix is required for installation to succeed.

For driver logs:

```sh
sudo waydroid logcat -- -d -s gpsd-binder gpsd-binder-relay gpsd-guard
# Or, for the selected emulator:
adb -s emulator-5554 logcat -d -s gpsd-binder gpsd-binder-relay
```

Both x86_64 and arm64 builds are available. The emulator installer has been tested on arm64
Android 13 and 17; the driver also has been tested on Android 16. x86_64 emulator testing is
still needed. The current two-program Waydroid build and installer still need a test together
on Waydroid.

For an unpacked release, run `sh waydroid/install-waydroid.sh` or
`sh android/install-emulator.sh --avd gpsd` inside it to use its binaries without downloading.
See [manual installation](docs/manual-installation.md) for file paths, diagnostics and recovery,
[other Android devices](docs/manual-installation.md#other-android-devices) for image integration,
and [BUILDING.md](BUILDING.md) to build from source. Physical devices are not supported by these
installers.
