# Manual installation of gpsd-binder

For normal Waydroid and emulator installations, use the scripts in
[INSTALLING.md](../INSTALLING.md). This reference describes the individual steps and recovery.
Run file-copy commands from an unpacked release archive, not from this documentation directory.

Choose [Android emulator](#android-emulator) or [Waydroid on Linux](#waydroid-on-linux).
For a physical device or a custom Android image, see [Other Android devices](#other-android-devices).
To build from source, see [BUILDING.md](../BUILDING.md).

## Requirements

- Android 13 or later. Android 12 and earlier cannot use this AIDL-only GNSS driver.
- x86_64 or arm64, matching the Android image, not necessarily the host.
- Root access to install files under `/vendor` and, where SELinux is enforcing, to install policy.
  Android 17 also needs a permission file under `/system`. Installing an APK is not enough.
- gpsd 3.20 or later, reachable from Android over TCP. gpsd keeps ownership of the receiver;
  installation does not require reconfiguring or restarting it.

The driver has two programs: the GNSS service and `gpsd-binder-relay`, which makes the TCP
connection to gpsd. Android's SELinux rules forbid HAL servers from using TCP sockets, so these
programs run in separate SELinux domains. Both run as Android's `gps` user (uid 1021). They link
only against the LLNDK libraries `libc.so`, `libdl.so` and `liblog.so`, not release-specific GNSS
libraries.

Tested configurations are arm64 AOSP emulator images for Android 13, 16 and 17 with SELinux
enforcing, and x86_64 Waydroid 1.6.3 with LineageOS 20 (Android 13). The two-program build and the
updated access guard have not yet been tested together on Waydroid. Physical arm64 devices,
x86_64 emulator images, and Android 14 and 15 have not been tested.

## Download and prepare the files

Each release on the [releases page](https://github.com/anelson/gpsd-binder/releases) has one archive
per architecture and a checksum file:

| Asset | Android architecture |
| --- | --- |
| `gpsd-binder-<version>-x86_64-linux-android.tar.gz` | x86_64 |
| `gpsd-binder-<version>-aarch64-linux-android.tar.gz` | arm64 (`arm64-v8a` in the SDK) |
| `SHA256SUMS` | checksums of both archives |

Download the archive for the Android image and `SHA256SUMS`, verify the checksum, and unpack it.
For example, on Linux:

```sh
sha256sum --check --ignore-missing SHA256SUMS
tar xzf gpsd-binder-<version>-x86_64-linux-android.tar.gz
cd gpsd-binder-<version>-x86_64-linux-android
```

On macOS, use `shasum -a 256` to compute the archive's checksum and compare it with `SHA256SUMS`.
On Windows, use PowerShell's `Get-FileHash -Algorithm SHA256` and `tar` to verify and unpack it.
The GitHub CLI can also verify the release's build provenance attestation:

```sh
gh attestation verify gpsd-binder-<version>-x86_64-linux-android.tar.gz --repo anelson/gpsd-binder
```

Run the installation commands from the unpacked directory. It contains:

| Path | Purpose |
| --- | --- |
| `android.hardware.gnss-service.gpsd-binder` | GNSS service |
| `gpsd-binder-relay` | TCP relay started by the service |
| `android/android.hardware.gnss-service.gpsd-binder.rc` | Android init service definition |
| `android/android.hardware.gnss-service.gpsd-binder.xml` | VINTF manifest fragment declaring `IGnss/default` |
| `android/gpsd-binder-permissions.xml` | local-network permission for uid `gps` on Android 17 and later |
| `android/sepolicy/` | SELinux policy, file and property contexts, and a policy check script |
| `waydroid/` | Linux host forwarder and Waydroid configuration and access guard |
| `LICENSE` | GPL-3.0-or-later |

Older archives without the relay and policy cannot use these instructions. Build both programs
from the current source instead; do not mix programs from different versions. For a source
build, use `target/<triple>/release/gpsd-binder` wherever the commands below use
`android.hardware.gnss-service.gpsd-binder`, and `target/<triple>/release/gpsd-binder-relay`
where they use `gpsd-binder-relay`. The `android/` and `waydroid/` files are in the source tree.

## Android emulator

This installs a real GNSS HAL in the Android Emulator supplied with the Android SDK. Apps use
Android's normal `gps` provider. Do not use the emulator's Location controls, `geo fix`, or any
mock-location tool; positions come from gpsd.

### 1. Create and boot a suitable virtual device

Install Android Studio, or the [SDK command-line tools](https://developer.android.com/studio#command-tools).
Use SDK Manager to install **Android Emulator**, **Android SDK Platform-Tools**, and an **AOSP**
(`default`) system image for API 33 or later. Do not choose a Google Play Store image: it does
not allow `adb root`. A Google APIs image without the Play Store may also allow root, but the
procedure below was tested with AOSP images.

Match the image to the host's processor for hardware acceleration, and download the corresponding
gpsd-binder archive:

| Emulator host | SDK image ABI | gpsd-binder archive architecture |
| --- | --- | --- |
| Apple silicon Mac | `arm64-v8a` | `aarch64-linux-android` |
| x86_64 Linux, Windows, or Intel Mac | `x86_64` | `x86_64-linux-android` |

The SDK emulator has no native Linux or Windows arm64 build. Use a supported host rather than
assuming the arm64 Android image makes those hosts supported.

Add the SDK's `platform-tools` and `emulator` directories to `PATH`; for command-line setup, also
add `cmdline-tools/latest/bin` and provide a compatible JDK. Android Studio manages the JDK for
its own SDK tools. Create a new, disposable AVD named `gpsd` in Device Manager. Enable its GPS
hardware, which is on by default. Alternatively, with the command-line tools (Apple silicon example):

```sh
sdkmanager "emulator" "platform-tools" "system-images;android-33;default;arm64-v8a"
avdmanager create avd --name gpsd --package "system-images;android-33;default;arm64-v8a"
```

Accept the SDK licences when prompted. Answer **no** (or press Enter) at the custom hardware
profile prompt. On an x86_64 host, replace `arm64-v8a` with `x86_64` in both commands. For another
Android release, use the exact package ID from `sdkmanager --list`; the tested Android 17 image
was `system-images;android-37.0;default;arm64-v8a`, including the `.0`.

Start the AVD from a terminal, not Android Studio's Quick Boot button:

```sh
emulator -avd gpsd -writable-system -no-snapshot
```

Keep these flags on every start. `-no-snapshot` forces a real boot rather than restoring old
framework or SELinux state. Leave SELinux enforcing; do not add `-selinux permissive`. For a
headless run, add `-no-window -no-audio`.

In a second terminal, run:

```sh
adb wait-for-device
adb root
adb wait-for-device
adb remount
```

If remount asks for a reboot, run `adb reboot`, wait for Android to finish booting, then repeat
`adb root`, `adb wait-for-device`, and `adb remount`. The first remount creates a writable overlay;
on newer images it also disables verity. Do not continue until remount succeeds. `adb wait-for-device`
waits for adbd, not the full Android boot; `adb shell getprop sys.boot_completed` must return `1`.

These commands assume only one connected Android device. If there are several, select this emulator
with `adb -s emulator-<port>` on every command, or set `ANDROID_SERIAL` in your host shell.

### 2. Install and load the SELinux policy first

Use two separate reboots: first load the policy while the emulator's original GNSS service still
works, then install and label the new programs. A file cannot reliably be labelled with a type
that the running policy does not yet know. Declaring the new HAL before init can start it can
leave Android waiting for GNSS during boot.

Check `adb shell ls -l /odm/etc/selinux`. The commands below assume no existing ODM policy or
context files. Some AOSP images ship an empty ODM policy and stock contexts; use the
[installer](../INSTALLING.md#android-emulator) for those images, which preserves the stock
contexts. **Do not overwrite existing ODM files with the commands below.** A nonempty unrelated
ODM policy needs image-specific integration as described under
[Other Android devices](#other-android-devices).

On the host:

```sh
adb shell mkdir -p -m 0755 /odm/etc/selinux
adb push android/sepolicy/sepolicy.cil /odm/etc/selinux/odm_sepolicy.cil
adb push android/sepolicy/file_contexts /odm/etc/selinux/odm_file_contexts
adb push android/sepolicy/property_contexts /odm/etc/selinux/odm_property_contexts
adb push android/sepolicy/check-policy.sh /data/local/tmp/check-policy.sh
adb shell
```

At the **Android shell prompt**, run:

```sh
cd /odm/etc/selinux
chown root:root odm_sepolicy.cil odm_file_contexts odm_property_contexts
chmod 0644 odm_sepolicy.cil odm_file_contexts odm_property_contexts
if [ -f /vendor/etc/selinux/precompiled_sepolicy ]; then
    mv /vendor/etc/selinux/precompiled_sepolicy /vendor/etc/selinux/precompiled_sepolicy.gpsd-binder-backup
fi
sh /data/local/tmp/check-policy.sh
exit
```

The script must report `SELinux policy checks passed`. It compiles the installed split policy
with Android's neverallow checks enabled. If it fails, do not reboot. Correct the policy, or
remove the three ODM files just installed and move the backed-up precompiled policy back to its
original name to undo this phase. Moving the precompiled policy aside makes init compile the CIL
at the next boot; otherwise init can use the old precompiled policy and ignore the added domains.
Init itself skips neverallow checks, which is why this separate check matters.

On the host:

```sh
adb reboot
```

Wait for the home screen and `sys.boot_completed=1`, then run `adb root`, `adb wait-for-device`
and `adb remount` again.

### 3. Install the programs and replace the emulator's GNSS service

On the host:

```sh
adb push android.hardware.gnss-service.gpsd-binder /vendor/bin/hw/android.hardware.gnss-service.gpsd-binder
adb push gpsd-binder-relay /vendor/bin/gpsd-binder-relay
adb push android/android.hardware.gnss-service.gpsd-binder.rc /vendor/etc/init/
adb push android/android.hardware.gnss-service.gpsd-binder.xml /vendor/etc/vintf/manifest/
adb shell
```

At the **Android shell prompt**:

```sh
cd /vendor
mkdir -p -m 0755 etc/gpsd-binder-backup/init etc/gpsd-binder-backup/manifest
mv etc/init/android.hardware.gnss*ranchu*.rc etc/gpsd-binder-backup/init/
mv etc/vintf/manifest/android.hardware.gnss*ranchu*.xml etc/gpsd-binder-backup/manifest/
printf 'on early-boot\n    setprop vendor.gpsd.host 10.0.2.2\n' > etc/init/gpsd-binder-emulator.rc
chown root:root bin/hw/android.hardware.gnss-service.gpsd-binder bin/gpsd-binder-relay
chown root:root etc/init/android.hardware.gnss-service.gpsd-binder.rc etc/init/gpsd-binder-emulator.rc etc/vintf/manifest/android.hardware.gnss-service.gpsd-binder.xml
chmod 0755 bin/hw/android.hardware.gnss-service.gpsd-binder bin/gpsd-binder-relay
chmod 0644 etc/init/android.hardware.gnss-service.gpsd-binder.rc etc/init/gpsd-binder-emulator.rc etc/vintf/manifest/android.hardware.gnss-service.gpsd-binder.xml
restorecon -v bin/hw/android.hardware.gnss-service.gpsd-binder bin/gpsd-binder-relay
ls -lZ bin/hw/android.hardware.gnss-service.gpsd-binder bin/gpsd-binder-relay
exit
```

The executable labels must be `hal_gnss_gpsd_binder_exec` and `gpsd_binder_relay_exec`, respectively.
Do not substitute `hal_gnss_default_exec`: the service and relay need their own domains.
The `ranchu` files belong to the emulator's original GNSS driver (HIDL on Android 13, AIDL on
newer images). Both its service definition and its manifest fragment must be moved out of init's
and VINTF's search directories. If the patterns match no files, stop and identify the image's
existing GNSS service rather than leaving two drivers declared.

The added init file points the relay at `10.0.2.2:2947`. In the emulator's default network,
[10.0.2.2 is an alias for the host's loopback](https://developer.android.com/studio/run/emulator-networking).
An existing gpsd listening on the host at `127.0.0.1:2947` is reachable there without changing its
configuration. If gpsd is elsewhere, use its reachable address instead. If it is in a container,
its TCP port must be published on the emulator host. Do not use `127.0.0.1` in Android to refer
to the host; it refers to Android itself.

**Android 17 (API 37) and later:** before this reboot, also install the native uid's local-network
permission. On the host:

```sh
adb push android/gpsd-binder-permissions.xml /system/etc/permissions/gpsd-binder.xml
adb shell
```

At the Android shell prompt:

```sh
chown root:root /system/etc/permissions/gpsd-binder.xml
chmod 0644 /system/etc/permissions/gpsd-binder.xml
restorecon /system/etc/permissions/gpsd-binder.xml
exit
```

Android 17 checks local-network access for native processes as well as apps. The `inet` group
and SELinux network domain do not grant that permission. The file explicitly grants both
`INTERNET` and `ACCESS_LOCAL_NETWORK` to uid `gps`. Without it, the relay times out on the
host's private address even though root can connect, with no SELinux denial. Granting only
`ACCESS_LOCAL_NETWORK` instead produces `Operation not permitted`, so keep both entries.
These grants are for uid `gps`, not for apps requesting location. Android 13 and 16 do not need
this file. It belongs under `/system/etc/permissions/`, not `/vendor`;
vendor permission files cannot assign permissions to native uids.

On the host, reboot once more:

```sh
adb reboot
```

### 4. Request a location and verify it

Install an app such as [GPSTest](https://github.com/barbeau/gpstest), for example with an APK you
have downloaded:

```sh
adb install -g /path/to/GPSTest.apk
```

Turn on Location in Android's Settings, open the app, and grant precise location permission if
asked. An app must request location; booting the driver alone does not start a gpsd connection.
The receiver needs a fix for positions to appear, but satellites may appear before a fix.
See [Check that it works](#check-that-it-works) for commands and expected results.

This is a test AVD, not a security boundary: other network-enabled apps in it may also be able
to reach the host's gpsd port without Android location permission. Use trusted apps and keep
gpsd off untrusted networks. The emulator procedure does not install the Waydroid-specific guard.

### Upgrade or remove the emulator installation

For a program-only upgrade, run `adb root` and `adb remount`, push **both** programs, then repeat
`chown`, `chmod` and `restorecon` on them and reboot. Keep the Android 17 permission file installed.
`adb push` can change ownership and labels, even when replacing an existing file. Policy updates need the policy check and policy reboot
before any new labels are applied.

To return to the original emulator driver, root and remount, then at the Android shell prompt:

```sh
cd /vendor
rm -f etc/init/android.hardware.gnss-service.gpsd-binder.rc etc/init/gpsd-binder-emulator.rc
rm -f etc/vintf/manifest/android.hardware.gnss-service.gpsd-binder.xml
rm -f bin/hw/android.hardware.gnss-service.gpsd-binder bin/gpsd-binder-relay
mv etc/gpsd-binder-backup/init/* etc/init/
mv etc/gpsd-binder-backup/manifest/* etc/vintf/manifest/
rm -f /odm/etc/selinux/odm_sepolicy.cil /odm/etc/selinux/odm_file_contexts /odm/etc/selinux/odm_property_contexts
if [ -f etc/selinux/precompiled_sepolicy.gpsd-binder-backup ]; then
    mv etc/selinux/precompiled_sepolicy.gpsd-binder-backup etc/selinux/precompiled_sepolicy
fi
rm -f /data/local/tmp/check-policy.sh
rm -f /system/etc/permissions/gpsd-binder.xml
exit
```

Reboot. Alternatively, delete this disposable AVD and create a new one. These removal commands
assume the fresh-AVD installation above, not an image with pre-existing ODM policy.

## Waydroid on Linux

Waydroid mounts `/var/lib/waydroid/overlay/vendor` on the Linux host over Android's `/vendor`.
The supplied integration leaves gpsd on host loopback and forwards connections through the
Waydroid bridge:

```text
receiver -> host gpsd (127.0.0.1:2947)
         -> host gpsd-waydroid-forward.service (192.168.240.1:2947)
         -> Android gpsd-binder-relay -> gpsd-binder -> Android gps provider
```

Waydroid has SELinux disabled, so **do not install the emulator's SELinux policy** there.
Use a build matching the Waydroid image, normally x86_64 on an x86_64 Linux host. Install `socat`
on the host with your distribution's package manager. The host also needs the legacy iptables
modules `ip_tables`, `iptable_filter`, `xt_owner` and `ipt_REJECT`; the forwarder loads them.

1. Stop the Android session as the desktop user:

   ```sh
   waydroid session stop
   ```

2. Check that `mount_overlays` in `/var/lib/waydroid/waydroid.cfg` is exactly `True`. Any other
   value, including `true`, disables the overlay. Remove any previously installed GNSS service
   and its VINTF declaration from the overlay so that only gpsd-binder is selected. If your
   underlying image supplies one, it must also be disabled; installing a second declaration
   does not replace it reliably.

3. From the unpacked archive, **as root on the Linux host**, install both programs and the
   Android configuration:

   ```sh
   O=/var/lib/waydroid/overlay/vendor
   install -o root -g root -m 0755 -D android.hardware.gnss-service.gpsd-binder "$O/bin/hw/android.hardware.gnss-service.gpsd-binder"
   install -o root -g root -m 0755 -D gpsd-binder-relay "$O/bin/gpsd-binder-relay"
   install -o root -g root -m 0644 -D android/android.hardware.gnss-service.gpsd-binder.rc "$O/etc/init/android.hardware.gnss-service.gpsd-binder.rc"
   install -o root -g root -m 0644 -D android/android.hardware.gnss-service.gpsd-binder.xml "$O/etc/vintf/manifest/android.hardware.gnss-service.gpsd-binder.xml"
   install -o root -g root -m 0644 -D waydroid/gpsd-binder-waydroid.rc "$O/etc/init/gpsd-binder-waydroid.rc"
   install -o root -g root -m 0755 -D waydroid/gpsd-access-guard.sh "$O/bin/gpsd-access-guard.sh"
   ```

   All files must be root-owned. init refuses a group- or world-writable `.rc` file.

4. Still as root, install the **host forwarder**. It is separate from the Android relay program:

   ```sh
   install -o root -g root -m 0644 waydroid/gpsd-waydroid-forward.service /etc/systemd/system/
   systemctl daemon-reload
   systemctl enable gpsd-waydroid-forward.service
   ```

   The unit starts when Waydroid creates `waydroid0` and stops when the bridge disappears. If
   the bridge already exists, also run `systemctl start gpsd-waydroid-forward.service` now.
   Do not change or restart gpsd; native Linux clients keep using its existing loopback socket.

5. Start the session as the desktop user:

   ```sh
   waydroid session start
   ```

   Overlay changes take effect at session start. Turn on Android Location and open an app that
   requests precise location, then follow [Check that it works](#check-that-it-works).

The init file sets `vendor.gpsd.host=192.168.240.1` and `vendor.gpsd.port=2947` and runs an
iptables access guard inside Android. Only sockets owned by uid `gps` (1021) may connect to
that address and port. The guard also covers a configured alternative IPv4 address and port.
It does not restrict host-side Linux clients. Other Android services with uid 1021 also have
access, and root can change the rules. Keep the guard and the service's `user gps` setting in
agreement, including when upgrading from an older root-running build.

If you changed Waydroid's bridge address or interface, update the integration files to match;
[waydroid/README.md](../waydroid/README.md) explains them. The forwarder binds to `waydroid0` as
well as its address so that it is not exposed through other host interfaces.

For diagnostics on the Linux host:

```sh
systemctl status gpsd-waydroid-forward.service
journalctl -u gpsd-waydroid-forward.service
sudo waydroid logcat -- -d -s gpsd-binder gpsd-binder-relay gpsd-guard
sudo waydroid shell
```

At the Android shell prompt, `iptables -S gpsd_guard` should show REJECT rules for uid ranges
`0-1020` and `1022-4294967294`. A guard error means Android apps may have direct access to gpsd;
fix it before treating Android's location permission as protection. The driver now logs to
logcat, not the host kernel log. Waydroid freezes inactive containers; no positions can be
forwarded while the container is frozen.

To upgrade, stop the session, repeat the file installation for both programs and the integration
files, reload the host unit if changed, and start the session. To uninstall, stop the session,
remove those six overlay files, disable the forwarder with
`systemctl disable --now gpsd-waydroid-forward.service`, remove its unit file, run
`systemctl daemon-reload`, and start the session. Do not remove unrelated overlay files.

## Configuration

The relay reads these system properties at every connection attempt, including reconnects:

| Property | Default | Meaning |
| --- | --- | --- |
| `vendor.gpsd.host` | `127.0.0.1` | gpsd's IP address or host name, as Android sees it |
| `vendor.gpsd.port` | `2947` | gpsd's TCP port |

Unset or empty properties use the defaults; invalid values are errors. Prefer a numeric IP
address: DNS from the relay's vendor SELinux domain has not been tested. Container and emulator
loopback addresses refer to Android, not to the host.

Set the address at boot in a root-owned, mode `0644` init file under `/vendor/etc/init/`, for example:

```text
on early-boot
    setprop vendor.gpsd.host 192.0.2.10
    setprop vendor.gpsd.port 2947
```

The two integration sections above install such a file. Changing properties in a root Android
shell with `setprop` takes effect at the next connection, not on an already open connection.
These are not persistent properties; edit the init file to keep a setting across boots.

Anything that can connect directly to gpsd can read positions without Android's location
permission and, unless gpsd is already read-only, can send commands that reconfigure the receiver.
Restrict access to that TCP port. Do not expose it on an untrusted network.

## Check that it works

Run these commands at a root Android shell prompt (`adb root` then `adb shell` for the emulator,
or `sudo waydroid shell` on Linux):

```sh
service list | grep gnss
logcat -d -s gpsd-binder gpsd-binder-relay
ps -AZ | grep gpsd
getprop vendor.gpsd.host
getprop vendor.gpsd.port
dumpsys location
```

Expected results:

- `service list` includes `android.hardware.gnss.IGnss/default`.
- On the emulator, `getenforce` returns `Enforcing`. The service runs in
  `u:r:hal_gnss_gpsd_binder:s0`; while an app requests location, its relay runs in
  `u:r:gpsd_binder_relay:s0`. Both have user `gps`. No relay while idle is normal.
- Once an app asks for location, logcat shows the relay connecting to the configured address,
  then the service reporting gpsd's version and devices.
- `dumpsys location` shows a request and last location for the normal `gps` provider, not a mock
  or test provider. GPSTest shows satellites and, when gpsd supplies a complete fix, a position.
  A fix needs a timestamp and horizontal accuracy (`eph`); the driver does not invent missing
  accuracy.

If the service is missing, check the VINTF fragment, executable ownership, modes and labels,
and reboot Android. `logcat -b all -d` includes service discovery and init errors. Repeated
"Waited one second for android.hardware.gnss.IGnss/default" usually means the HAL is declared
but init cannot start or register it; it can also prevent boot from completing. adbd may still
be available during that wait: root, remount, correct the executable labels with `restorecon`,
and reboot. If recovery is not practical, delete the disposable AVD and start again.

If the relay cannot connect, its log line gives the address and reason; the service may also
report a reset connection. Check routing, the listening port, host firewall, and any access guard.
On Android 17, a timeout without a denial can also mean the local-network permission file is
missing; check `/system/etc/permissions/gpsd-binder.xml` and reboot after installing it.
For SELinux failures, inspect `dmesg | grep 'avc: denied'` and `logcat -b all -d` for the two
process domains. Do not work around them by disabling SELinux or granting TCP access to a HAL
domain; Android's neverallow rules prohibit that design.

## Other Android devices

A physical device needs a rooted, writable vendor partition or a suitable overlay, and a policy
integrated with that device's Android image. The two programs install at:

| File | Installed path | Mode |
| --- | --- | --- |
| `android.hardware.gnss-service.gpsd-binder` | `/vendor/bin/hw/android.hardware.gnss-service.gpsd-binder` | `0755` |
| `gpsd-binder-relay` | `/vendor/bin/gpsd-binder-relay` | `0755` |
| `android/android.hardware.gnss-service.gpsd-binder.rc` | `/vendor/etc/init/android.hardware.gnss-service.gpsd-binder.rc` | `0644` |
| `android/android.hardware.gnss-service.gpsd-binder.xml` | `/vendor/etc/vintf/manifest/android.hardware.gnss-service.gpsd-binder.xml` | `0644` |

All must be root-owned. On Android 17 and later, also install
`android/gpsd-binder-permissions.xml` as `/system/etc/permissions/gpsd-binder.xml` (root-owned,
mode `0644`) to grant local-network access to uid `gps`. This needs a writable system partition
or overlay as well as vendor access.

Disable the existing GNSS service and its VINTF declaration, install the policy and configuration,
label the programs, and reboot. Android selects its GNSS HAL at boot;
simply copying a binary into a running system does not replace the current driver.

`android/sepolicy/` supplies CIL policy, executable file contexts, and property contexts for
`vendor.gpsd.host` and `vendor.gpsd.port`. A device that already has ODM policy needs these merged
with its existing files, not copied over them. Its precompiled policy must also be rebuilt or
disabled, including any copy under `/odm/etc/selinux/`. A policy change must be loaded before
labelling files with newly defined types. Keep recovery access and backups before changing a
physical device's boot policy.

The supplied CIL uses unversioned platform type names to work on the tested built emulator
images. A device-tree integration needs the equivalent source policy in its vendor policy build,
with the platform's versioned interfaces and Treble checks. The image's framework compatibility
matrix must also accept GNSS AIDL version 2. The emulator procedure is not a tested general-purpose
installer for physical devices or arbitrary vendor images.
