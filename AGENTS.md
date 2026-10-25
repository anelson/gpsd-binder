# Notes for agents

This file is for agents who work on gpsd-binder. README.md says what the
project is, BUILDING.md how to build it, and INSTALLING.md how to install it. This file records the
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

## Build and check

BUILDING.md says what to install and what each `just` recipe does. The rules
for agents:

- `just fmt`, `just vibecheck` and `just depcheck` must pass completely clean,
  with no warnings, before a change is presented. `just build` fails if the
  binary needs any library other than `libc.so` and `libdl.so`.
- The check tools (`taplo`, `ast-grep`, `typos`, `cargo-deny`, `cargo-machete`) run
  through `cgx`, the user's own tool; their versions are in `cgx.toml`. Do not
  install them by hand or call them directly in the justfile. CI also runs
  `just` itself through cgx (`cgx just <recipe>`), which is why `cgx.toml`
  lists it; locally the user runs `just` directly.
- `rust-toolchain.toml` pins Rust 1.89.0; `rust-version` in `Cargo.toml` says
  the same. The clippy lint names in `Cargo.toml` and the `#[expect]` list on
  the generated bindings are tied to that version (1.89 has already dropped
  cgx's `match_on_vec_items`; 1.96 renames `empty_enum` and drops
  `string_to_string`). The driver's real floor is 1.86, set by rsbinder 0.12;
  going there needs cargo's MSRV-aware resolver, `ignore` pinned to 0.4.29 in
  the lock, and testcontainers 0.25.
- The lints in `Cargo.toml`, `clippy.toml` and the ast-grep rules are the cgx
  project's. Suppress a lint with `#[expect(..., reason = "...")]`, never
  `#[allow]`; `unsafe_code` is forbidden (the boot-time clock comes from
  `rustix`). Comments and Markdown use ASCII punctuation only.
- Unit tests live in `#[cfg(test)] mod tests` at the bottom of the file they
  test; there is no `tests/` directory. `just test` runs them on the host,
  including the gpsd module's test against a real gpsd and gpsfake in a
  container. The driver binary itself cannot run on the host; the HAL's
  session logic needs a device.
- Do not write hand-rolled parsers for standard formats; gpsd's time stamps
  are `chrono::DateTime<Utc>`. Nothing is stringly typed where it is up to us:
  gpsd's JSON is deserialized into the types in `src/gpsd/protocol.rs`, and
  the WATCH request is serialized from `Watch`.
- Do not build the driver binary for plain Linux to run it: rsbinder then
  speaks only the Android 16 servicemanager protocol (see below). rsbinder and
  rsbinder-aidl do compile on macOS and Linux, which is what lets the unit
  tests and the verify build of `cargo package` run on the host.
- arm64 (`just build arm64`) builds, but has not run on a device.
- The NDK supplies only the linker, the C runtime start files, `libunwind.a`
  and the stub libraries; rustc compiles the code. The API level in the
  linker name (30), not the NDK version, sets the oldest Android release that
  can load the binary. CI uses NDK r30, the 2026 long-term-support release;
  Rust supports the newest LTS NDK. Do not go back to r27: from Android 15 on,
  arm64 devices may use 16 KiB pages, which cannot load a binary aligned to
  4 KiB, and only r28 and later align to 16 KiB by default.

## CI and releases

BUILDING.md describes CI and releases for people. The facts for agents:

- Workflows: `.github/workflows/ci.yml` (checks, tests, `cargo package`, and
  builds for both targets), `release.yml` (runs on a tag `v*`; started by
  hand it only builds and packages), and `dependabot-auto-merge.yml`. Every
  job runs on `ubuntu-26.04`. No arm runner is used: the NDK has no Linux
  arm64 host toolchain, and both targets cross-compile from x86_64.
- The required status check is the job named `Build and Test` in `ci.yml`.
  The branch protection on master names it, so renaming the job blocks every
  pull request.
- Rust comes from `rustup toolchain install --no-self-update`, which reads
  `rust-toolchain.toml`. Everything else comes through the cgx action,
  referenced as `anelson/cgx@master` with its defaults (latest release,
  caching on) and `prefetch-all`. It is unpinned on purpose: the user updates
  cgx often and wants each release at once. Do not pin it, turn off its
  cache, or replace it with per-tool install actions.
- The NDK comes from the runner image, named by version in `ANDROID_NDK_HOME`
  in both workflows (`/usr/local/lib/android/sdk/ndk/30.0.16248370`). When an
  image drops that version, the build fails with "linker not found"; change
  both workflows to a version that the image's README lists.
- `ci.yml` uses version tags for actions. `release.yml` pins them by commit
  hash, because its jobs can publish. Dependabot updates both kinds.
- Security model: pull requests from forks run only after the maintainer
  approves the run; the CI token is read-only; no workflow uses a secret or a
  personal access token. Do not add `pull_request_target` or `workflow_run`
  triggers. In `release.yml` the job that runs build tools has a read-only
  token, and the jobs that hold publishing permissions run no build tools.
  The crates.io job therefore publishes with `--no-verify`; the build job has
  already run `cargo package --locked`.
- crates.io trusted publishing accepts tokens only from the workflow file
  `release.yml`, in the GitHub environment `release`. Renaming either breaks
  publishing until the crates.io settings change to match. crates.io accepts
  a trusted publisher only for a crate that exists, so the first version is
  published by hand with a token.
- Dependabot opens weekly pull requests, after a 7-day cooldown, with minor
  and patch updates grouped. `dependabot-auto-merge.yml` turns on auto-merge
  for each of them; the branch protection on master holds the merge until
  the user has approved the pull request and `Build and Test` has passed. A
  merge made with the workflow token starts no workflows, so master has no CI
  run for such a merge commit.
- Releases: `.agents/skills/release/SKILL.md`. The release notes are the
  version's section of `CHANGELOG.md`.
- Repository settings that the workflows depend on: fork pull request
  approval for all external contributors; auto-merge allowed; classic branch
  protection on master, as in cgx, that requires one approving review (stale
  approvals dismissed) and the `Build and Test` check from GitHub Actions, and
  does not bind admins; a ruleset on tags `v*` that lets only repository
  admins create, move or delete them; the environment `release`, deployable
  only from tags `v*`.
- Skills live in `.agents/skills/`; `.claude/skills` is a symlink to it, for
  Claude Code.
- Never add a `Co-Authored-By` trailer for an AI agent to a commit message or
  a pull request description. The hook in `.claude/settings.json` blocks such
  commits, and the same file turns off Claude Code's own attribution.

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
- NMEA: `GnssNmeaProvider` calls `startNmea()` when the first app NMEA
  listener becomes active and `stopNmea()` when the last one stops being
  active, before or after `start()`, and calls `startNmea()` again after the
  driver dies. A listener stays active while its app is in the foreground or
  runs a foreground service. ATAK runs one, so another app in front does not
  stop NMEA; GPSTest has a listener while its screen is shown.
- The JNI drops NMEA sentences while no app listens only for HIDL and AIDL
  version 1 drivers. From a version 2 driver every `gnssNmeaCb()` goes up to
  Java, so the driver sends sentences only between `startNmea()` and
  `stopNmea()`.
- The framework copies at most 120 bytes of each `gnssNmeaCb()` sentence
  (`GnssNmeaProvider.mNmeaBuffer`). Apps get the timestamp unchanged, as
  `OnNmeaMessageListener`'s Unix time in milliseconds.
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
- With `"nmea":true` in the WATCH, gpsd sends NMEA sentences on the same
  connection as the JSON: bare lines with no JSON around them, each ending in
  CR LF as the JSON lines do, the sentences of a packet before the reports
  made from it. Sentences from an NMEA receiver are copied as received. For a
  binary receiver, such as the CF-20's u-blox, gpsd builds `$GP` sentences
  itself (`gpsd/pseudonmea.c`): RMC, GSA and GSV without a fix, GGA and ZDA
  only with one. Comment lines are not sent. Other text packets (AIS
  `!AIVDM`, Garmin `@`) are copied too; the driver takes only lines that
  begin with `$` as NMEA, so it logs each such line as one that is not a
  report.
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
  the port named by the property. The driver reads the property only when it
  connects. To make it switch at once, kill it by PID; init restarts it and
  the framework renews the active request within a second. Stopping the unit
  closes the driver's connection, and it reconnects to the port the property
  names next. A stopped transient unit stays listed as failed until
  `systemctl reset-failed gpsd-binder-fake2948`.
- Run `am force-stop`, `setprop` and `svc power stayon true` as separate `wsh`
  calls. One `waydroid shell` command that chained them stalled for about a
  minute on the stay-on setting, so its `am force-stop` ran after the app had
  been launched and killed it. Launch apps with
  `waydroid app launch <package>` as the session user with `XDG_RUNTIME_DIR`,
  `WAYLAND_DISPLAY` and `DBUS_SESSION_BUS_ADDRESS` set. ATAK
  (`com.atakmap.app.civ`) needs one to seven minutes after a session start
  before it requests location; it shows its splash screen until then.
- ATAK 5.8 registers an NMEA listener with its location request. When a
  location has an altitude, ATAK replaces it with field 9 (height above sea
  level) of the last `$GPGGA`, `$GNGGA` or `$GLGGA` sentence, converted to
  height above the ellipsoid with EGM96. Without GGA sentences it shows no
  altitude.
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
  gpsd when the test forwarder goes away, recovery after an app force-stop,
  and recovery after the driver is killed with a request active. The build
  installed on 2026-10-25 was linked with NDK r30. GPSTest received the
  driver's NMEA sentences, and `startNmea()` and `stopNmea()` followed the
  listeners of ATAK and GPSTest (2026-10-25, without a fix, so without GGA).
- Not yet tested: a real sky fix, a long run, freeze and thaw, gpsd restarts,
  arm64, Android versions other than 13. The stale-gap logic is unit-tested
  with a fake clock but not exercised by a real freeze.
