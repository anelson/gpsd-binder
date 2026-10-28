# gpsd-binder on Waydroid

These files connect gpsd on a Linux host to gpsd-binder in a
[Waydroid](https://waydro.id/) container. They have been tested with
Waydroid 1.6.3, LineageOS 20 (Android 13) and Fedora 44. gpsd's own
configuration does not change.

Data path: receiver -> gpsd on the host (127.0.0.1:2947) ->
`gpsd-waydroid-forward.service` on the Waydroid bridge (192.168.240.1:2947) ->
gpsd-binder-relay in Android -> gpsd-binder -> Android's `gps` location provider.

| File | Installed as | Purpose |
|---|---|---|
| `gpsd-waydroid-forward.service` | host: `/etc/systemd/system/` | accepts connections on the bridge and relays them to gpsd; loads the iptables modules that the guard needs |
| `gpsd-binder-waydroid.rc` | Android: `/vendor/etc/init/` | sets the driver's gpsd address to 192.168.240.1:2947 and runs the guard |
| `gpsd-access-guard.sh` | Android: `/vendor/bin/` | resets TCP connections to gpsd from every Android uid except gps (1021) |

## Install

Use the [Waydroid installer](../INSTALLING.md#waydroid-on-linux). It detects the Android
architecture and bridge address, installs the files, and restarts the session. Run it as your
desktop user with Waydroid running. From an unpacked release, use `sh waydroid/install-waydroid.sh`.

For the individual file-copy commands, see
[manual installation](../docs/manual-installation.md#waydroid-on-linux).

## Notes

- 192.168.240.1 is the host's address on Waydroid's bridge, set by
  `LXC_ADDR` in `/usr/lib/waydroid/data/scripts/waydroid-net.sh`. If you
  change it, change it in all three files.
- The guard lets only uid gps (1021) connect. Both the driver and its relay
  run as that uid, as the driver's init file specifies. Other services with
  the same uid also have access; root can change the rules.
- Waydroid has SELinux disabled, so it needs no policy installation. Read
  the driver's logs with `waydroid logcat -- -d -s gpsd-binder gpsd-binder-relay gpsd-guard`.
- Waydroid freezes the container about 60 s after the last Android activity
  (`suspend_action = freeze`). While it is frozen, no positions reach Android.
