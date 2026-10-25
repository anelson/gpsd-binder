# Notes for agents

This file is for agents who work on gpsd-binder. README.md says
what the project is and how to build and install it. This file records the
requirements, the facts that cost effort to learn, and how to work on the
code and on a test device.

## Requirements

Do not violate these. Ask the user before doing anything that comes close.

- Android apps must get real positions through Android's own GNSS stack: the
  driver feeds Android's `GnssLocationProvider`, and apps see the normal
  `gps` provider.
- No mock location, ever, not even for testing. Do not use or propose
  `LocationManager` test providers, Appium's location helper, mock-location
  apps or anything else that injects positions through the mock-location API.
- gpsd owns the receiver. Do not reconfigure gpsd or the receiver, and do not
  restart gpsd on the user's machines. Native Linux clients of gpsd must keep
  working.
- The driver runs inside Android. It makes no assumptions about where gpsd
  runs or how it is reached, apart from a TCP address. A design in which the
  driver runs on the Linux host and registers through a container's binder
  device was considered and rejected.
- The driver is generic. Waydroid is one integration, kept in `waydroid/`.
  Do not put Waydroid assumptions in `src/` or `android/`, and do not present
  the project as Waydroid-specific.
- Target Android 13 (SDK 33) and later (LineageOS 20 or later with Waydroid).
- The program must link only against LLNDK libraries, so that one build
  serves several Android releases. Today its only `NEEDED` entries are
  `libc.so` and `libdl.so`. `liblog.so` is also LLNDK and would be
  acceptable.
- The user prefers Rust.
- Write in plain technical English. Do not invent jargon and do not
  anthropomorphize software. Match the comment density and idiom of the
  surrounding code.

## Layout

| Path          | Contents                                                                                                                                    |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/main.rs` | what is specific to the Android system: properties, the boot-time clock, logging to stderr                                                  |
| `src/hal.rs`  | the `IGnss` service, its sessions, and the mapping from gpsd's reports to the AIDL types; unit tests of the mapping                          |
| `src/gpsd/`   | the gpsd client: `protocol.rs` (report and request types), `connection.rs`, `feed.rs` (reconnecting loop); `testdata/` for the container test |
| `build.rs`    | generates Rust bindings from the frozen AIDL files                                                                                          |
| `aidl/`       | frozen `android.hardware.gnss` version 2 AIDL API and its hash, from AOSP (Apache-2.0); never edit                                          |
| `android/`    | the driver's init file and VINTF manifest fragment, for `/vendor` on any Android                                                            |
| `waydroid/`   | Waydroid integration: address-setting init file, access guard, host forwarder unit                                                          |
| `justfile`    | the check recipes; `*.toml`, `sgconfig.yml` and `.ast-grep/` configure them (copied from the user's cgx project); `rust-toolchain.toml` pins the toolchain |

## Build and check

```
rustup target add x86_64-linux-android
ANDROID_NDK_HOME=/path/to/android-ndk-r27c scripts/build.sh x86_64 30   # or: just build
```

- `just fmt`, `just vibecheck` and `just depcheck` must pass completely clean,
  with no warnings, before a change is presented. `vibecheck` runs ast-grep,
  typos, `cargo check` and `cargo clippy -D warnings` for the host and for
  `x86_64-linux-android`, and `cargo doc` with warnings as errors. `depcheck`
  runs cargo-deny and cargo-machete. The tools (`just`, `taplo`, `ast-grep`,
  `typos`, `cargo-deny`, `cargo-machete`) come from Homebrew or `cargo install`;
  `fmt` also needs a nightly toolchain for `rustfmt-nightly.toml`.
- `rust-toolchain.toml` pins Rust 1.89.0 and installs clippy, rustfmt,
  rust-analyzer and the Android target; `rust-version` in `Cargo.toml` says
  the same. The clippy lint names in `Cargo.toml` and the `#[expect]` list on
  the generated bindings are tied to that version (1.89 has already dropped
  cgx's `match_on_vec_items`; 1.96 renames `empty_enum` and drops
  `string_to_string`). The
  driver's real floor is 1.86, set by rsbinder 0.12; going there needs
  cargo's MSRV-aware resolver, `ignore` pinned to 0.4.29 in the lock, and
  testcontainers 0.25.
- The lints in `Cargo.toml`, `clippy.toml` and the ast-grep rules are the cgx
  project's. Suppress a lint with `#[expect(..., reason = "...")]`, never
  `#[allow]`; `unsafe_code` is forbidden (the boot-time clock comes from
  `rustix`). Comments and Markdown use ASCII punctuation only.
- Unit tests live in `#[cfg(test)] mod tests` at the bottom of the file they
  test; there is no `tests/` directory. `just test` runs them on the host,
  including the gpsd module's test against a real gpsd and gpsfake in a
  container (Docker, about 5 s once the image is built). The driver binary
  itself cannot run on the host; the HAL's session logic needs a device.
- Do not write hand-rolled parsers for standard formats; gpsd's time stamps
  are `chrono::DateTime<Utc>`. Nothing is stringly typed where it is up to us:
  gpsd's JSON is deserialized into the types in `src/gpsd/protocol.rs`, and
  the WATCH request is serialized from `Watch`.
- The NDK in use is r27c (27.2.12479018). On macOS the `.dmg` holds the NDK
  root at `AndroidNDK<build>.app/Contents/NDK`. The NDK ships universal
  binaries under `toolchains/llvm/prebuilt/darwin-x86_64`, which
  `scripts/build.sh` uses on both Intel and Apple silicon.
- The linker is `x86_64-linux-android30-clang`. API level 30 lets the binary
  load on Android 11 and later; the AIDL interface needs Android 13 anyway.
- A release build takes about 20 s from clean and gives a stripped PIE of
  about 1 MB. `cargo check` and `cargo clippy` for the Android target do not
  need the NDK; only linking does.
- After a build, check the dependencies:
  `$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-readelf -d target/x86_64-linux-android/release/gpsd-binder | grep NEEDED`
  must list only `libc.so` and `libdl.so`.
- Do not build the driver binary for plain Linux: rsbinder then speaks only
  the Android 16 servicemanager protocol (see below). rsbinder and
  rsbinder-aidl do compile on macOS and Linux, which is what lets the unit
  tests run on the host.
- arm64 (`scripts/build.sh arm64 30`) has not been built or tested.

## Android 13 GNSS framework

LineageOS 20 source is the reference. Read it at
`https://raw.githubusercontent.com/LineageOS/<repo>/lineage-20.0/<path>`.
The relevant files are in `android_frameworks_base` under
`services/core/java/com/android/server/location/gnss/` (`GnssLocationProvider.java`,
`GnssNative.java`), `services/core/jni/gnss/` and
`services/core/jni/com_android_server_location_GnssLocationProvider.cpp`.

- Probing: system_server looks for a GNSS driver once, at boot, with
  `waitForVintfService`. If an AIDL driver is declared and reports interface
  version 2 or more, it uses AIDL only and never asks for HIDL. A driver must
  be declared in a VINTF manifest fragment and running at boot; after
  installing one, restart Android.
- Android 12 and earlier cannot use an AIDL-only driver: AIDL version 1
  carried only extensions.
- servicemanager checks only that an instance is declared. It does not check
  the framework compatibility matrix at run time, so an AIDL driver works from
  a vendor image with an older target level. Real devices check the matrix at
  build time.
- Extensions: return an error status such as `UnsupportedOperation` from each
  `getExtension*()`. Returning success with no object makes the framework
  wrap the empty object, and it can crash later.
- `setCallback()` is called at boot (as an init and close cycle), when
  Location is turned on, on a user switch, for bypass requests such as
  emergency calls, and after the driver dies. The framework ignores its
  result. `close()` is called when Location is turned off.
- The framework resets its record of capabilities only at boot and after the
  driver dies. Later calls add bits but never remove them. Capabilities must
  therefore not depend on gpsd's state.
- Without `SCHEDULING`, the framework always passes 1000 ms to
  `setPositionMode`. For request intervals over 10 s it stops the driver after
  each fix and restarts it by alarm; for 60 s or more it also stops it after
  60 s without a fix. gpsd keeps the receiver running, so this saves no power
  and costs a reconnect per cycle.
- With `SCHEDULING`, `setPositionMode` carries the real interval and
  `lowPowerMode`, at each start and whenever the request changes.
  `preferredAccuracyMeters` and `preferredTimeMs` are always 0.
- `start()` can be called twice without a `stop()` in between.
- `SESSION_BEGIN` produces `onStarted` for apps; `SESSION_END` or
  `ENGINE_OFF` produce `onStopped`; `ENGINE_ON` is ignored.
- The framework drops deliveries that arrive faster than each app asked for.
- Capability bits not to report: MSB, MSA, SINGLE_SHOT, ON_DEMAND_TIME (it
  starts time injection), LOW_POWER_MODE, and every bit that needs an
  extension (geofencing, measurements, navigation messages, satellite
  blocklist, measurement corrections, antenna info, correlation vectors,
  satellite PVT).
- When servicemanager restarts, its init file restarts every class `hal`
  service, so the driver is restarted and registers again. No re-registration
  inside the process is needed.
- When the driver dies, the framework logs "gnss hal died - restarting
  shortly", probes again and calls `setCallback()`.
- The vendor linker namespace is isolated. The HIDL GNSS libraries exist only
  in `/system/lib64`, and the VNDK APEX has only
  `android.hardware.gnss-V2-ndk.so`. A program that needs only LLNDK
  libraries avoids this.

## rsbinder 0.12.0

- It is pinned (`=0.12.0`, pre-1.0) with `default-features = false` (no
  tokio) and the `android_13_plus` feature.
- Built for Android, it reads `ro.build.version.sdk` at run time and uses
  the matching servicemanager protocol, for SDK 33 to 37.
- Built for any other target, it compiles only the Android 16 servicemanager
  client. Android 16 added methods before `addService`, so the method numbers
  differ from Android 13's.
- `rsbinder-aidl`'s `.version()` and `.hash()` apply only to the most
  recently added source, so `build.rs` stamps each file. Android reads
  `getInterfaceVersion()` and uses AIDL only at version 2 or more.
- `@VintfStability` in the AIDL makes rsbinder mark binder objects VINTF.
  Calls across the system and vendor boundary need that mark.
- Cargo forbids dots in program names. The program is `gpsd-binder`; install
  it as `/vendor/bin/hw/android.hardware.gnss-service.gpsd-binder`.
- The generated bindings trip `unreachable_pub`, `clippy::allow_attributes`
  and `clippy::unused_self` under this crate's lint set; `mod gen` in
  `src/hal.rs` expects exactly those. If a toolchain or rsbinder-aidl update
  changes the set, clippy reports an unfulfilled expectation.
- Do not link a death notification to servicemanager (handle 0). Once it
  fires, rsbinder's cached servicemanager proxy stays dead for the life of the
  process. `reconnect.rs` only helps clients find a service again; it does not
  register services.

## gpsd

Behaviour verified in gpsd 3.27.5's source (`gpsd/gpsd.c`,
`man/gpsd_json.adoc`):

- gpsd keeps a receiver open only while some client is watching, unless it
  runs with `-n`. It closes the device 60 s after the last watcher goes
  (`RELEASE_TIMEOUT`). Reopening resets its driver state but does not reset
  the receiver.
- A client that disconnects and a client that sends
  `?WATCH={"enable":false};` have the same effect, so the driver simply
  closes the connection on `stop()`.
- gpsd answers `?WATCH` with only the device list and the watch settings, no
  stored reports. A new connection therefore carries only new data.
- A connected client that is not watching and sends nothing is dropped after
  15 minutes. Request lines are limited to 80 characters.
- When a client's socket is full, gpsd drops reports, and disconnects the
  client after a partial write.
- Without `-b` (read-only mode), any client can send commands that
  reconfigure the receiver. Anything that can connect can also read raw
  positions without Android's location permission.
- Under socket activation gpsd accepts at most the control socket plus two TCP
  listeners. Fedora's `gpsd.socket` already passes three sockets, so an extra
  `ListenStream` is silently ignored; hence the separate forwarder.
- gpsd 3.25 writes `"readonly":"true"` as a string in DEVICE, against its own
  specification; the driver does not model that field. TPVs derived from GLL
  sentences carry a position but no `time`; the driver forwards only fixes
  with a time. The driver reads `altHAE` only (gpsd 3.20 and later) and
  `eph` only; it does not fall back to the deprecated `alt` or to `epx`.
- gpsfake: `-o` must be written `-o=-G` for gpsd to listen on all addresses;
  without `-1` it replays the log forever; it prints "log cycle of ...
  begins." to stderr once gpsd is up.
- On Fedora with SELinux enforcing, `/usr/bin/gpsd` cannot bind other ports
  (such as 2948), and `systemd-socket-proxyd` cannot bind ports at all. A test
  gpsd must run from a copy of the binary outside `/usr/bin`; the forwarder
  uses `socat`.

Mistakes made by earlier drivers fed from gpsd, to avoid:

- numbering satellites from `PRN` instead of `svid` (GLONASS then shows as
  65-96);
- inventing an accuracy when gpsd gives none;
- reporting altitude above sea level instead of above the ellipsoid
  (`altHAE`), or dropping fractional seconds;
- a zero `ElapsedRealtime`;
- replaying stale fixes after a freeze;
- extension getters that return success with no object;
- clearing the callback on `stop()`, or never reconnecting.

## Waydroid (version 1.6.3, LineageOS 20 image)

How the integration works: gpsd on the host listens on loopback;
`waydroid/gpsd-waydroid-forward.service` relays connections from the bridge
address 192.168.240.1:2947 to it; `waydroid/gpsd-binder-waydroid.rc` points
the driver there; `waydroid/gpsd-access-guard.sh` resets connections to gpsd
from every Android uid except 0.

- Overlay: `/var/lib/waydroid/overlay` is mounted over the system image and
  `/var/lib/waydroid/overlay/vendor` over the vendor image, read-only, when a
  session starts. Changes take effect at the next session start. Files must be
  root-owned, and init refuses group- or world-writable `.rc` files.
  Waydroid's tools never delete `overlay/`.
- `mount_overlays` in `/var/lib/waydroid/waydroid.cfg` must be exactly the
  string `True`; any other value, including `true`, disables the overlay.
  Waydroid writes `False` itself if mounting the system overlay fails.
- Properties under `[properties]` in `waydroid.cfg` reach Android only after
  `waydroid upgrade -o`, which restarts the container. Set properties from an
  init file instead (`on early-boot`).
- Device nodes in the container config are regenerated by `waydroid init`,
  `waydroid upgrade` and package updates, and `lxc-device add` fails. The TCP
  design avoids device nodes.
- The container is privileged. Android's SELinux is disabled. On the host,
  Android processes run as `container_runtime_t`.
- Init cannot create per-service cgroups ("Read-only file system"), so
  `setprop ctl.restart <service>` leaves a service stuck in "stopping", and
  class restarts hang. Restart a service by killing its PID; init starts it
  again. After a servicemanager restart, restart the whole session.
- The host is 192.168.240.1 on the bridge `waydroid0` (`LXC_ADDR` in
  `waydroid-net.sh`); dnsmasq gives the container 192.168.240.112.
- Linux accepts packets for a local address on any interface. A listener
  merely bound to 192.168.240.1 is reachable from other networks; the
  forwarder uses `so-bindtodevice=waydroid0`, plus `IPAddressAllow` and
  `IPAddressDeny`.
- netd flushes its own child chains, `oem_out` included, when it starts, but
  keeps rules that others add directly to `OUTPUT`. The guard therefore keeps
  its own chain with one jump at the top of `OUTPUT`. It matches
  `--uid-owner 1-4294967294`, not `! --uid-owner 0`: a negated match also
  catches the ownerless packets the kernel sends after a socket closes, which
  left the forwarder's sockets retransmitting for minutes.
- The guard needs the legacy iptables modules (`ip_tables`, `iptable_filter`,
  `xt_owner`, `ipt_REJECT`), which Android cannot load. The forwarder unit
  loads them before it starts.
- Waydroid freezes the whole container about 60 s after the last Android
  activity (`suspend_action = freeze`). While it is frozen nothing runs,
  including the driver.
- The images are ext4. `debugfs -R "cat /system/build.prop" <images_path>/system.img`
  reads the system properties without starting Waydroid; `images_path` is in
  `waydroid.cfg`.

Operating a Waydroid container from scripts:

- `waydroid shell` thaws a frozen container, runs the command and freezes it
  again. It always exits with status 0, so do not rely on its exit status.
  Writing straight to an SSH channel makes it fail with
  `[Errno 13] Permission denied: 1`; pipe its output through `cat`.
- It is slow (a Python start-up per call). The equivalent direct command is
  `lxc-attach -P /var/lib/waydroid/lxc -n waydroid --clear-env` plus the
  environment variables that Waydroid's `tools/helpers/lxc.py` sets.
  `lxc-attach` into a frozen container blocks forever: check
  `lxc-info -P /var/lib/waydroid/lxc -n waydroid -sH` first, and wrap every
  attach in `timeout` with stdin from `/dev/null`.
- During tests run `svc power stayon true` inside Android, and
  `svc power stayon false` as the last step.
- Logs: `waydroid logcat -- -d -v time -b all` (the `--` is required).
  Android init messages and anything a service writes with `stdio_to_kmsg`
  appear in the host kernel log (`journalctl -k`).
- Screenshots: `screencap -p /data/local/tmp/s.png` inside Android; the file
  appears on the host under `~/.local/share/waydroid/data/local/tmp/`. Delete
  it afterwards.
- Starting a session from a script as the desktop user needs
  `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY` and `DBUS_SESSION_BUS_ADDRESS`. Poll
  `getprop sys.boot_completed`; boot takes 12 to 20 s.
- Do not use `adb connect`. It raises an authorization dialog inside Android
  that injected taps cannot dismiss. If it appears, close it with
  `am broadcast -a android.hardware.usb.action.USB_STATE --ez connected false`.
- Kill processes by PID. An unanchored `pkill -f` pattern matches the SSH
  command line and other people's processes.

## Testing on a device

The user's test machine is a Panasonic CF-20 tablet running Fedora 44 with
Waydroid 1.6.3 and the LineageOS 20 image (Android 13, x86_64), with a u-blox
NEO-M8N receiver owned by gpsd. Ask the user for its address and
credentials. Never write credentials into the repository. SSH is by password
(`sshpass -e` with `SSHPASS`); `sudo` takes the same password through
`sudo -S -p ''`.

- Since 2026-10-24 gpsd-binder is the installed driver on the CF-20; the
  earlier HIDL HAL (bzobl/gpsd) is moved aside under
  `/home/rupert/waydroid-gps/gpsd-binder/replaced-hidl-<timestamp>/`, with a
  full overlay backup in `/home/rupert/waydroid-gps/backup/`. That directory
  also holds `install.sh` (checksum, stop session, back up, swap files, start
  session) and a README with the way back.
- To install a new build: stop the session, copy the binary over
  `/var/lib/waydroid/overlay/vendor/bin/hw/android.hardware.gnss-service.gpsd-binder`
  (root-owned, 0755) and the init and VINTF files if they changed, and start
  the session. `/home/rupert/waydroid-gps/final/common.sh` has
  `remember_session`, `stop_session` and `start_session`.
- Helpers on the device: `mechanics/wsh 'cmd'` runs one command as root inside
  Android; `hostgps/start_gpsfake_2948.sh` and `stop_gpsfake_2948.sh` run a
  private gpsd through gpsfake on 127.0.0.1:2948 with a synthetic track near
  Greenwich (a copy of the gpsd binary outside `/usr/bin`, because of
  SELinux); `final/status.sh` is the health check, adapted to gpsd-binder.
- Without a sky view, replay a recording: start gpsfake on 2948, expose it
  with a temporary `socat` unit bound to `waydroid0`
  (`systemd-run --unit=gpsd-binder-fake2948 -p BindsTo=sys-subsystem-net-devices-waydroid0.device -p After=sys-subsystem-net-devices-waydroid0.device -p DynamicUser=yes /usr/bin/socat TCP-LISTEN:2948,bind=192.168.240.1,so-bindtodevice=waydroid0,freebind,reuseaddr,fork TCP:127.0.0.1:2948`),
  then `setprop service.gpsd.port 2948` inside Android. The guard also covers
  the port named by the property. Stopping the unit closes the driver's
  connection, and it reconnects to the port the property names next.
- Run `am force-stop`, `setprop` and `svc power stayon true` as separate `wsh`
  calls. One `waydroid shell` command that chained them stalled for about a
  minute on the stay-on setting, so its `am force-stop` ran after the app had
  been launched and killed it. Launch apps with
  `waydroid app launch <package>` as the session user with `XDG_RUNTIME_DIR`,
  `WAYLAND_DISPLAY` and `DBUS_SESSION_BUS_ADDRESS` set. ATAK
  (`com.atakmap.app.civ`) needs about a minute before it requests location.
- Success looks like this: servicemanager logs "Found
  android.hardware.gnss.IGnss/default in device VINTF manifest"; `lshal` shows
  no HIDL GNSS service and `service list` shows
  `android.hardware.gnss.IGnss/default`; the kernel log shows
  `gpsd-binder: session N: connected to gpsd 3.27.5 at ...` and the device
  list; `dumpsys location` shows the `gps` provider's request and last
  location and no mock or test provider; ATAK shows the position in its
  bottom-right readout.
- Before ending: the port property back at 2947, the temporary unit and
  gpsfake stopped, `svc power stayon false`, ATAK running, and
  `final/status.sh` reporting no problems.
- Proved on the device: registration and selection by the framework after a
  session restart, live satellites from the receiver, a replayed fix into
  ATAK, switching the port property at run time, reconnecting to the real
  gpsd when the test forwarder goes away, and recovery after an app
  force-stop.
- Not yet tested: a real sky fix, a long run, freeze and thaw, gpsd restarts,
  arm64, Android versions other than 13. The stale-gap logic is unit-tested
  with a fake clock but not exercised by a real freeze.
