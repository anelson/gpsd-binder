# Installing gpsd-binder

These instructions are for installing a release build. To build from source, see BUILDING.md; the result installs
the same way.

## Requirements

- Android 13 or later. Android 12 and earlier cannot use a GNSS driver that implements only the
  AIDL interface. Only Android 13 has been tested.
- x86_64 or arm64. My use case is on an x86_64 Linux laptop with Waydroid, I haven't personally tested the arm64 builds on a real Android device, but it should work. Probably. Maybe.
- SELinux disabled or permissive. There is no SELinux policy for the driver yet. Waydroid runs
  Android with SELinux disabled.
- A way to put files under `/vendor`. On a device, `/vendor` is the vendor partition, so this means
  a rooted device whose vendor partition can be remounted, or a vendor overlay such as Waydroid's.
- gpsd 3.20 or later, reachable from Android over TCP. The driver does not need to run where gpsd
  runs; see [Configuration](#configuration).

The binary links only against `libc.so` and `libdl.so`, so it does not depend on the libraries of
one Android release.

## Download

Each release on the [releases page](https://github.com/anelson/gpsd-binder/releases) has one
archive per architecture and a checksum file:

| Asset                                                | Contents                                      |
| ---------------------------------------------------- | --------------------------------------------- |
| `gpsd-binder-<version>-x86_64-linux-android.tar.gz`  | the driver for x86_64 Android, with its files |
| `gpsd-binder-<version>-aarch64-linux-android.tar.gz` | the same for arm64 Android                    |
| `SHA256SUMS`                                         | checksums of both archives                    |

Download the archive for your architecture and the checksum file, check the archive, and unpack it:

```sh
sha256sum --check --ignore-missing SHA256SUMS
tar xzf gpsd-binder-<version>-x86_64-linux-android.tar.gz
cd gpsd-binder-<version>-x86_64-linux-android
```

Each archive also has a build provenance attestation, which shows that the release workflow in
this repository built it. With the GitHub CLI:

```sh
gh attestation verify gpsd-binder-<version>-x86_64-linux-android.tar.gz --repo anelson/gpsd-binder
```

The directory holds:

| Path                                                    | Purpose                                                    |
| ------------------------------------------------------- | ---------------------------------------------------------- |
| `android.hardware.gnss-service.gpsd-binder`             | the driver                                                 |
| `android/android.hardware.gnss-service.gpsd-binder.rc`  | its init file                                              |
| `android/android.hardware.gnss-service.gpsd-binder.xml` | its VINTF manifest fragment, which declares it to Android  |
| `waydroid/`                                             | the Waydroid integration (see below); not needed elsewhere |
| `LICENSE`                                               | GPL-3.0-or-later                                           |

## Install on Android

Copy the three files into `/vendor`:

| File                                                    | Installed as                                                               | Mode   |
| ------------------------------------------------------- | -------------------------------------------------------------------------- | ------ |
| `android.hardware.gnss-service.gpsd-binder`             | `/vendor/bin/hw/android.hardware.gnss-service.gpsd-binder`                 | `0755` |
| `android/android.hardware.gnss-service.gpsd-binder.rc`  | `/vendor/etc/init/android.hardware.gnss-service.gpsd-binder.rc`            | `0644` |
| `android/android.hardware.gnss-service.gpsd-binder.xml` | `/vendor/etc/vintf/manifest/android.hardware.gnss-service.gpsd-binder.xml` | `0644` |

All three must be owned by root. init refuses an `.rc` file that is group- or world-writable.

If the system already has a GNSS driver, remove its init file and VINTF fragment first, or Android
has two drivers declared and uses whichever it finds first. A driver that implements the older
HIDL interface and `gpsd-binder` can not both be active.

Then restart Android. Android looks for a GNSS driver once, while it boots, so a driver installed
into a running system is not used until the next boot.

## Configuration

The driver connects to gpsd at the address in two system properties, read each time Android starts
a GNSS session:

| Property            | Default     | Meaning                                            |
| ------------------- | ----------- | -------------------------------------------------- |
| `service.gpsd.host` | `127.0.0.1` | gpsd's IP address or host name, as Android sees it |
| `service.gpsd.port` | `2947`      | gpsd's TCP port                                    |

The defaults will work automatically when gpsd runs on the same system as Android. To use another address, add
an init file of your own under `/vendor/etc/init/` that sets the properties before the driver
starts:

```
on early-boot
    setprop service.gpsd.host 192.168.240.1
    setprop service.gpsd.port 2947
```

Changing a property on a running system takes effect at the driver's next connection, for example
when the next app asks for location.

Anything that can open a TCP connection to gpsd can read positions without Android's location
permission, and without gpsd's `-b` option it can also send commands that reconfigure the receiver.
Where that matters, restrict which processes can connect. The Waydroid integration does this with
an iptables rule that admits only the driver's uid.

## Install on Waydroid

Waydroid mounts `/var/lib/waydroid/overlay/vendor` on the host over Android's `/vendor`. gpsd runs
on the host, and the files in `waydroid/` connect the two: a systemd unit that relays connections
from the Waydroid bridge to gpsd, an init file that points the driver at that relay, and the access
rule. `waydroid/README.md` explains each.

On the host, as root:

1. Check that `mount_overlays` in `/var/lib/waydroid/waydroid.cfg` is exactly `True`. Any other
   value, including `true`, disables the overlay.
2. Copy the files into the overlay, root-owned, with the modes above:

   ```sh
   O=/var/lib/waydroid/overlay/vendor
   install -o root -g root -m 0755 -D android.hardware.gnss-service.gpsd-binder "$O/bin/hw/android.hardware.gnss-service.gpsd-binder"
   install -o root -g root -m 0644 -D android/android.hardware.gnss-service.gpsd-binder.rc "$O/etc/init/android.hardware.gnss-service.gpsd-binder.rc"
   install -o root -g root -m 0644 -D android/android.hardware.gnss-service.gpsd-binder.xml "$O/etc/vintf/manifest/android.hardware.gnss-service.gpsd-binder.xml"
   install -o root -g root -m 0644 -D waydroid/gpsd-binder-waydroid.rc "$O/etc/init/gpsd-binder-waydroid.rc"
   install -o root -g root -m 0755 -D waydroid/gpsd-access-guard.sh "$O/bin/gpsd-access-guard.sh"
   ```

3. Install the relay. It needs `socat`, and it loads the iptables modules that the access rule in
   Android needs:

   ```sh
   install -o root -g root -m 0644 waydroid/gpsd-waydroid-forward.service /etc/systemd/system/
   systemctl daemon-reload
   systemctl enable gpsd-waydroid-forward.service
   ```

   The unit starts when Waydroid creates its bridge and stops when the bridge goes. gpsd's own
   configuration does not change, and Linux programs keep using it as before.

4. Restart the Waydroid session: `waydroid session stop`, then `waydroid session start`. Overlay
   changes take effect at the next session start.

The driver's log lines go to the host's kernel log: `journalctl -k | grep gpsd-binder`.

## Check that it works

Inside Android (`adb shell`, or `waydroid shell` on the host):

- `logcat -d | grep 'android.hardware.gnss.IGnss/default'` shows servicemanager finding the
  driver in the device VINTF manifest, and `GnssJni` reporting "Successfully got GNSS AIDL
  handle. Version=2".
- `service list | grep gnss` lists `android.hardware.gnss.IGnss/default`.
- `dumpsys location` shows a `gps` provider. Once an app has asked for location, it shows the
  provider's request and, when gpsd has a fix, its last location. There is no mock or test
  provider.
- An app such as GPSTest shows gpsd's satellites, with the same numbers, elevations and azimuths as
  `cgps` on the gpsd side, and a fix when the receiver has one.

If servicemanager does not find the driver, the VINTF fragment is missing or Android did not
restart. If it finds the driver but apps get nothing, the kernel log shows what the driver
connected to and what gpsd answered.

## Upgrade

Replace the driver binary with the new one and restart Android. The init file and the VINTF
fragment change rarely, but it won't hurt to copy those over as well.

## Uninstall

Remove the files you installed and restart Android. On Waydroid, also
`systemctl disable --now gpsd-waydroid-forward.service` and remove the unit file.
