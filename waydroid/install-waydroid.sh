#!/bin/sh
set -eu

fail() { echo "Error: $*" >&2; exit 1; }
usage() {
    echo "Usage: sh $0 [--host ADDRESS] [--port PORT]"
    echo "Run as your desktop user with Waydroid running. Installs into Waydroid Android 13."
    echo "By default, forwards the host's loopback gpsd through the Waydroid bridge."
}
host=
port=2947
while [ "$#" -gt 0 ]; do
    case "$1" in
        --help|-h) usage; exit 0 ;;
        --host|--port)
            [ "$#" -ge 2 ] || fail "$1 needs a value"
            case "$1" in --host) host=$2 ;; --port) port=$2 ;; esac
            shift 2 ;;
        *) usage >&2; fail "Unknown option: $1" ;;
    esac
done
case "$host" in -*|*[!a-zA-Z0-9.:-]*) fail "Invalid gpsd address: $host" ;; esac
case "$port" in ''|*[!0-9]*) fail "Invalid port: $port" ;; esac
[ "$port" -ge 1 ] && [ "$port" -le 65535 ] || fail "Port must be between 1 and 65535"
[ "$(uname -s)" = Linux ] || fail "Waydroid installation requires Linux"
[ "$(id -u)" -ne 0 ] || fail "Run this as your desktop user, without sudo. The script requests sudo when needed."
[ -n "${XDG_RUNTIME_DIR:-}" ] && [ -n "${WAYLAND_DISPLAY:-}" ] || fail "Run this from a terminal in your Wayland desktop session."
for tool in waydroid sudo timeout ip findmnt install systemctl modprobe getent; do
    command -v "$tool" >/dev/null 2>&1 || fail "Required command is missing: $tool"
done
if [ -z "$host" ] && ! command -v socat >/dev/null 2>&1; then
    if command -v dnf >/dev/null 2>&1; then hint='sudo dnf install socat'
    elif command -v apt-get >/dev/null 2>&1; then hint='sudo apt-get install socat'
    elif command -v pacman >/dev/null 2>&1; then hint='sudo pacman -S socat'
    else hint="your distribution's package manager"; fi
    fail "socat is missing. Install it with: $hint"
fi
waydroid status | grep -Eq 'Session:[[:space:]]+RUNNING' || fail "Start your Waydroid session first, then rerun this script."
sudo -v
android() { sudo timeout 20 waydroid shell "$@" | tr -d '\r'; }
[ "$(android getprop sys.boot_completed)" = 1 ] || fail "Waydroid has not finished booting. Wait for the home screen, then rerun."
[ "$(android getprop ro.build.version.sdk)" = 33 ] || fail "This installer supports Waydroid Android 13 (LineageOS 20)."
case "$(android getprop ro.product.cpu.abi)" in
    arm64-v8a) triple=aarch64-linux-android ;;
    x86_64) triple=x86_64-linux-android ;;
    *) fail "Unsupported Android architecture: $(android getprop ro.product.cpu.abi)" ;;
esac
[ "$(android getenforce)" = Disabled ] || fail "This Waydroid installer expects SELinux disabled; it will not change SELinux."
root=/var/lib/waydroid/rootfs
overlay=/var/lib/waydroid/overlay/vendor
mounts=$(findmnt -n -o FSTYPE,OPTIONS --mountpoint "$root/vendor") || fail "Waydroid vendor image is not mounted at $root/vendor"
case "$mounts" in overlay*"$overlay"*) ;; *) fail "Waydroid overlays are not active. Set mount_overlays = True in /var/lib/waydroid/waydroid.cfg and restart Waydroid." ;; esac
network=$(ip -4 -o address show dev waydroid0 | awk '{print $4}')
[ -n "$network" ] && [ "$(printf '%s\n' "$network" | wc -l)" -eq 1 ] || fail "Expected one IPv4 address on waydroid0"
bridge=${network%/*}
subnet=$(ip -4 route show dev waydroid0 proto kernel scope link | awk 'NR == 1 {print $1}')
[ -n "$subnet" ] || fail "No IPv4 subnet route found for waydroid0"
remote=0
if [ -n "$host" ]; then
    host=$(getent ahostsv4 "$host" | awk 'NR == 1 {print $1}')
    [ -n "$host" ] || fail "The gpsd host must resolve to an IPv4 address for the access guard."
    case "$host" in 127.*|0.0.0.0) fail "--host is an address reached from Android. Omit --host to use Linux host loopback." ;; esac
    remote=1
else
    host=$bridge
fi
# An older third-party driver is not part of the supported empty Waydroid setup.
for dir in "$root/vendor/etc/init" "$root/vendor/etc/vintf" "$root/odm/etc/vintf"; do
    [ -d "$dir" ] || continue
    files=$(sudo grep -rl --include='*.rc' --include='*.xml' android.hardware.gnss "$dir") || [ "$?" -eq 1 ]
    for file in $files; do
        case "$file" in */android.hardware.gnss-service.gpsd-binder.rc|*/android.hardware.gnss-service.gpsd-binder.xml) ;;
            *) fail "Existing GNSS configuration: $file. Remove that older installation first." ;;
        esac
    done
done
sudo modprobe -a ip_tables iptable_filter xt_owner ipt_REJECT || fail "The host kernel lacks the iptables modules needed by the access guard."
work=$(mktemp -d)
phase="preparing the release"
cleanup() {
    result=$?
    rm -rf "$work"
    if [ "$result" -ne 0 ]; then
        if [ "$remote" -eq 0 ] && [ "$phase" != 'preparing the release' ]; then
            sudo systemctl stop gpsd-waydroid-forward.service 2>/dev/null || true
        fi
        echo "Installation stopped while $phase." >&2
    fi
}
trap cleanup 0
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
payload=$here
[ -f "$payload/android.hardware.gnss-service.gpsd-binder" ] || payload=$(dirname "$here")
if [ ! -f "$payload/android.hardware.gnss-service.gpsd-binder" ]; then
    for tool in curl tar; do command -v "$tool" >/dev/null 2>&1 || fail "$tool is missing"; done
    release=$(curl -fsSL -o /dev/null -w '%{url_effective}' https://github.com/anelson/gpsd-binder/releases/latest)
    tag=${release##*/}
    case "$tag" in v[0-9]*) ;; *) fail "Could not find the latest gpsd-binder release" ;; esac
    archive=gpsd-binder-${tag#v}-$triple.tar.gz
    url=https://github.com/anelson/gpsd-binder/releases/download/$tag
    echo "Downloading $archive..."
    curl -fLsS "$url/SHA256SUMS" -o "$work/SHA256SUMS"
    awk -v file="$archive" '$2 == file' "$work/SHA256SUMS" >"$work/checksum"
    [ -s "$work/checksum" ] || fail "Release $tag has no archive for $triple"
    curl -fLsS "$url/$archive" -o "$work/$archive"
    (cd "$work"; if command -v sha256sum >/dev/null 2>&1; then sha256sum -c checksum; else shasum -a 256 -c checksum; fi)
    tar -xzf "$work/$archive" -C "$work"
    payload=$work/${archive%.tar.gz}
fi
[ -f "$payload/gpsd-binder-relay" ] && [ -f "$payload/waydroid/gpsd-access-guard.sh" ] ||
    fail "This archive is missing the relay or access guard. Use a current release."
machine=$(od -An -tu1 -j18 -N2 "$payload/android.hardware.gnss-service.gpsd-binder" | tr -s ' ' | xargs)
case "$triple:$machine" in aarch64-linux-android:183\ 0|x86_64-linux-android:62\ 0) ;; *) fail "The local archive does not match Android's $triple architecture" ;; esac
sed -e "s/192\.168\.240\.1/$host/g" -e "s/2947/$port/g" "$payload/waydroid/gpsd-access-guard.sh" >"$work/gpsd-access-guard.sh"
sed -e "s/192\.168\.240\.1/$host/g" -e "s/2947/$port/g" "$payload/waydroid/gpsd-binder-waydroid.rc" >"$work/gpsd-binder-waydroid.rc"
sed -e "s/192\.168\.240\.1/$bridge/g" -e "s|192\.168\.240\.0/24|$subnet|g" -e "s/2947/$port/g" \
    "$payload/waydroid/gpsd-waydroid-forward.service" >"$work/gpsd-waydroid-forward.service"

phase="installing the Waydroid files"
echo "Stopping the Waydroid session..."
if [ -f /etc/systemd/system/gpsd-waydroid-forward.service ]; then
    sudo systemctl stop gpsd-waydroid-forward.service
fi
timeout 30 waydroid session stop || fail "Could not stop Waydroid. Close it and rerun this script."
sudo install -o root -g root -m 0755 -D "$payload/android.hardware.gnss-service.gpsd-binder" "$overlay/bin/hw/android.hardware.gnss-service.gpsd-binder"
sudo install -o root -g root -m 0755 -D "$payload/gpsd-binder-relay" "$overlay/bin/gpsd-binder-relay"
sudo install -o root -g root -m 0644 -D "$payload/android/android.hardware.gnss-service.gpsd-binder.rc" "$overlay/etc/init/android.hardware.gnss-service.gpsd-binder.rc"
sudo install -o root -g root -m 0644 -D "$payload/android/android.hardware.gnss-service.gpsd-binder.xml" "$overlay/etc/vintf/manifest/android.hardware.gnss-service.gpsd-binder.xml"
sudo install -o root -g root -m 0644 -D "$work/gpsd-binder-waydroid.rc" "$overlay/etc/init/gpsd-binder-waydroid.rc"
sudo install -o root -g root -m 0755 -D "$work/gpsd-access-guard.sh" "$overlay/bin/gpsd-access-guard.sh"
if [ "$remote" -eq 0 ]; then
    sudo install -o root -g root -m 0644 "$work/gpsd-waydroid-forward.service" /etc/systemd/system/gpsd-waydroid-forward.service
    sudo systemctl daemon-reload
    sudo systemctl enable gpsd-waydroid-forward.service
else
    # Close an earlier local forwarder when switching this installation to a remote receiver.
    if [ -f /etc/systemd/system/gpsd-waydroid-forward.service ]; then
        sudo systemctl disable --now gpsd-waydroid-forward.service
    fi
fi
phase="starting Waydroid and checking the driver"
log=${XDG_RUNTIME_DIR:?Run from your desktop session}/gpsd-binder-session.log
echo "Starting the Waydroid session (log: $log)..."
waydroid session start >"$log" 2>&1 &
deadline=$(($(date +%s) + 180))
until [ "$(android getprop sys.boot_completed 2>/dev/null)" = 1 ]; do
    [ "$(date +%s)" -lt "$deadline" ] || fail "Waydroid did not boot within three minutes. Check $log"
    sleep 2
done
rules=$(android iptables -S gpsd_guard)
if ! printf '%s\n' "$rules" | grep -q -- '--uid-owner 0-1020' ||
    ! printf '%s\n' "$rules" | grep -q -- '--uid-owner 1022-4294967294' ||
    ! android sh -c 'iptables -C OUTPUT -j gpsd_guard && echo guard-jump-ok' | grep -Fxq guard-jump-ok; then
    fail "The gpsd access guard did not load. The local forwarder has not been left running. Check: sudo waydroid logcat -- -d -s gpsd-guard"
fi
if [ "$remote" -eq 0 ]; then sudo systemctl restart gpsd-waydroid-forward.service; fi
android service list | grep -F android.hardware.gnss.IGnss/default || fail "GNSS service did not register. Check: sudo waydroid logcat -- -d -s gpsd-binder gpsd-binder-relay"
echo "Installed. Using gpsd at $host:$port. Enable Android Location and open ATAK or another location app."
