# gpsd-binder on Waydroid

These files connect gpsd on a Linux host to gpsd-binder in a
[Waydroid](https://waydro.id/) container. They have been tested with
Waydroid 1.6.3, LineageOS 20 (Android 13) and Fedora 44. gpsd's own
configuration does not change.

Data path: receiver → gpsd on the host (127.0.0.1:2947) →
`gpsd-waydroid-forward.service` on the Waydroid bridge (192.168.240.1:2947) →
gpsd-binder in Android → Android's `gps` location provider.

| File | Installed as | Purpose |
|---|---|---|
| `gpsd-waydroid-forward.service` | host: `/etc/systemd/system/` | accepts connections on the bridge and relays them to gpsd; loads the iptables modules that the guard needs |
| `gpsd-binder-waydroid.rc` | Android: `/vendor/etc/init/` | sets the driver's gpsd address to 192.168.240.1:2947 and runs the guard |
| `gpsd-access-guard.sh` | Android: `/vendor/bin/` | resets TCP connections to gpsd from every Android uid except 0 |

## Install

1. Build the driver and install its files (see the top-level README), with
   these two Android files, under `/var/lib/waydroid/overlay/vendor/` on the
   host. Waydroid mounts that directory over `/vendor`. The files must be
   owned by root, and the `.rc` files must not be group- or world-writable.
2. Install the forwarder:
   `systemctl daemon-reload; systemctl enable gpsd-waydroid-forward.service`.
3. Restart the Waydroid session. Overlay changes take effect at the next
   session start.

## Notes

- 192.168.240.1 is the host's address on Waydroid's bridge, set by
  `LXC_ADDR` in `/usr/lib/waydroid/data/scripts/waydroid-net.sh`. If you
  change it, change it in all three files.
- The guard lets only uid 0 connect, so the driver must run as root, as the
  driver's init file currently specifies.
- Waydroid freezes the container about 60 s after the last Android activity
  (`suspend_action = freeze`). While it is frozen, no positions reach Android.
