#!/bin/sh
set -eu

fail() { echo "Error: $*" >&2; exit 1; }
usage() {
    echo "Usage: sh $0 [--avd NAME | --serial SERIAL] [--host ADDRESS] [--port PORT]"
    echo "Installs into an AOSP Android 13, 16 or 17 emulator. Requires adb and, with --avd, emulator."
}
host=10.0.2.2
port=2947
avd=
serial=${ANDROID_SERIAL:-}
while [ "$#" -gt 0 ]; do
    case "$1" in
        --help|-h) usage; exit 0 ;;
        --avd|--serial|--host|--port)
            [ "$#" -ge 2 ] || fail "$1 needs a value"
            case "$1" in
                --avd) avd=$2; serial= ;;
                --serial) serial=$2 ;;
                --host) host=$2 ;;
                --port) port=$2 ;;
            esac
            shift 2 ;;
        *) usage >&2; fail "Unknown option: $1" ;;
    esac
done
case "$host" in ''|-*|*[!a-zA-Z0-9.:-]*) fail "Invalid gpsd address: $host" ;; esac
case "$port" in ''|*[!0-9]*) fail "Invalid port: $port" ;; esac
[ "$port" -ge 1 ] && [ "$port" -le 65535 ] || fail "Port must be between 1 and 65535"
[ -z "$avd" ] || [ -z "$serial" ] || fail "Choose --avd or --serial, not both"
command -v adb >/dev/null 2>&1 || fail "adb is missing. Install Android SDK Platform-Tools and add platform-tools to PATH."
work=$(mktemp -d)
phase="checking the emulator"
cleanup() {
    result=$?
    rm -rf "$work"
    if [ "$result" -ne 0 ]; then echo "Installation stopped while $phase." >&2; fi
}
trap cleanup 0
adb start-server >/dev/null
emulators=$(adb devices | awk '$1 ~ /^emulator-[0-9]+$/ {print $1}')
if [ -n "$avd" ]; then
    case "$avd" in *[!a-zA-Z0-9_.-]*) fail "Invalid AVD name: $avd" ;; esac
    for device in $emulators; do
        if adb -s "$device" emu avd name 2>/dev/null | tr -d '\r' | grep -Fxq "$avd"; then
            [ -z "$serial" ] || fail "More than one running emulator uses AVD $avd"
            serial=$device
        fi
    done
    if [ -z "$serial" ]; then
        command -v emulator >/dev/null 2>&1 || fail "emulator is missing. Add the SDK emulator directory to PATH."
        emulator -list-avds | grep -Fxq "$avd" || fail "AVD $avd does not exist. Create an AOSP AVD in Android Studio first."
        console=5554
        while printf '%s\n' "$emulators" | grep -Fxq "emulator-$console"; do console=$((console + 2)); done
        serial=emulator-$console
        log=${TMPDIR:-/tmp}/gpsd-binder-$avd.log
        echo "Starting $avd (log: $log)..."
        emulator -avd "$avd" -port "$console" -writable-system -no-snapshot >"$log" 2>&1 &
    fi
elif [ -z "$serial" ]; then
    [ "$(printf '%s\n' "$emulators" | grep -c .)" -eq 1 ] || fail "Select an emulator with --avd NAME or --serial emulator-PORT."
    serial=$emulators
fi
case "$serial" in emulator-*[!0-9]*|emulator-) fail "Not an emulator serial: $serial" ;; emulator-*) ;; *) fail "Refusing physical device $serial" ;; esac
ad() { adb -s "$serial" "$@"; }
prop() { ad shell getprop "$1" | tr -d '\r'; }
wait_boot() {
    tries=0
    until [ "$(ad get-state 2>/dev/null)" = device ] && [ "$(prop sys.boot_completed 2>/dev/null)" = 1 ]; do
        tries=$((tries + 1))
        [ "$tries" -lt 120 ] || fail "$serial did not finish booting within four minutes. Check its emulator log."
        sleep 2
    done
}
reboot() { ad reboot; sleep 2; wait_boot; }
root_remount() {
    ad root
    tries=0
    until [ "$(ad shell id -u 2>/dev/null | tr -d '\r')" = 0 ]; do
        tries=$((tries + 1))
        [ "$tries" -lt 10 ] || fail "This image does not allow adb root. Use an AOSP image, not a Play Store image."
        sleep 1
    done
    result=$(ad remount 2>&1) || fail "adb remount failed: $result"
    echo "$result"
    case "$result" in *reboot*) reboot; ad root; sleep 2; ad remount ;; esac
    ad shell 'touch /vendor/etc/init/.gpsd-binder-write-test && rm /vendor/etc/init/.gpsd-binder-write-test' ||
        fail "Vendor is not writable. Restart this AVD with -writable-system -no-snapshot, then rerun."
}
echo "Checking $serial..."
wait_boot
[ "$(prop ro.kernel.qemu)" = 1 ] || fail "$serial is not an Android emulator"
case "$(prop ro.product.name)" in sdk_phone*) ;; *) fail "Only AOSP SDK images are supported. Create an AOSP (default) AVD." ;; esac
sdk=$(prop ro.build.version.sdk)
case "$sdk" in 33|36|37) ;; *) fail "Android API $sdk is not supported by this installer; use AOSP API 33, 36 or 37." ;; esac
case "$(prop ro.product.cpu.abi)" in
    arm64-v8a) triple=aarch64-linux-android ;;
    x86_64) triple=x86_64-linux-android ;;
    *) fail "Unsupported Android architecture: $(prop ro.product.cpu.abi)" ;;
esac

phase="preparing the release"
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
payload=$here
[ -f "$payload/android.hardware.gnss-service.gpsd-binder" ] || payload=$(dirname "$here")
if [ ! -f "$payload/android.hardware.gnss-service.gpsd-binder" ]; then
    for tool in curl tar; do command -v "$tool" >/dev/null 2>&1 || fail "$tool is missing"; done
    release=$(curl -fsSL -o /dev/null -w '%{url_effective}' https://github.com/anelson/gpsd-binder/releases/latest)
    tag=${release##*/}
    case "$tag" in v[0-9]* ) ;; *) fail "Could not find the latest gpsd-binder release" ;; esac
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
[ -f "$payload/gpsd-binder-relay" ] && [ -f "$payload/android/sepolicy/sepolicy.cil" ] ||
    fail "This archive is missing the relay or policy. Use a current release."
# ELF e_machine distinguishes the two release architectures, including local archives.
machine=$(od -An -tu1 -j18 -N2 "$payload/android.hardware.gnss-service.gpsd-binder" | tr -s ' ' | xargs)
case "$triple:$machine" in aarch64-linux-android:183\ 0|x86_64-linux-android:62\ 0) ;; *) fail "The local archive does not match Android's $triple architecture" ;; esac

phase="checking the installed policy and original GNSS service"
root_remount
[ "$(ad shell getenforce | tr -d '\r')" = Enforcing ] || fail "Expected an AOSP image with SELinux enforcing."
for name in sepolicy.cil file_contexts property_contexts; do
    case "$name" in sepolicy.cil) dest=odm_sepolicy.cil ;; *) dest=odm_$name ;; esac
    current=$work/$name
    : >"$current"
    if ad shell "test -e /odm/etc/selinux/$dest"; then
        ad exec-out cat "/odm/etc/selinux/$dest" >"$current"
    fi
    if [ "$name" = sepolicy.cil ]; then
        [ -z "$(tr -d '[:space:]' <"$current")" ] ||
            cmp -s "$current" "$payload/android/sepolicy/$name" ||
            fail "Existing nonempty ODM policy differs from ours. Use a fresh AOSP AVD."
    else
        # Keep stock AOSP contexts; append our entries only when they are absent.
        complete=1
        while IFS= read -r line; do
            grep -Fxq "$line" "$current" || complete=0
        done <"$payload/android/sepolicy/$name"
        if [ "$complete" -eq 0 ]; then
            ! grep -q gpsd "$current" || fail "Conflicting gpsd entries in $dest"
            printf '\n' >>"$current"
            cat "$payload/android/sepolicy/$name" >>"$current"
        fi
    fi
done
ad shell 'test ! -e /odm/etc/selinux/precompiled_sepolicy' || fail "Existing ODM precompiled policy found. Use a fresh AOSP AVD."
ad shell sh <<'ANDROID'
set -eu
found=0
for dir in /vendor/etc/init /vendor/etc/vintf /odm/etc/vintf; do
    [ -d "$dir" ] || continue
    for file in $(grep -rl android.hardware.gnss "$dir" || true); do
        case "$file" in
            */android.hardware.gnss*ranchu*.rc|*/android.hardware.gnss*ranchu*.xml|*/android.hardware.gnss-service.gpsd-binder.rc|*/android.hardware.gnss-service.gpsd-binder.xml) found=1 ;;
            *) echo "Unrecognized GNSS configuration: $file" >&2; exit 1 ;;
        esac
    done
done
[ "$found" = 1 ] || { echo "No recognized emulator GNSS configuration found" >&2; exit 1; }
ANDROID
phase="installing SELinux policy"
echo "Installing and checking policy..."
ad shell mkdir -p -m 0755 /data/local/tmp/gpsd-binder
for file in sepolicy.cil file_contexts property_contexts check-policy.sh; do
    source=$payload/android/sepolicy/$file
    case "$file" in file_contexts|property_contexts) source=$work/$file ;; esac
    ad push "$source" "/data/local/tmp/gpsd-binder/$file" >/dev/null
done
ad shell sh /data/local/tmp/gpsd-binder/check-policy.sh /data/local/tmp/gpsd-binder/sepolicy.cil
ad shell 'set -eu
mkdir -p -m 0755 /odm/etc/selinux
cd /odm/etc/selinux
cp /data/local/tmp/gpsd-binder/sepolicy.cil odm_sepolicy.cil
cp /data/local/tmp/gpsd-binder/file_contexts odm_file_contexts
cp /data/local/tmp/gpsd-binder/property_contexts odm_property_contexts
chown root:root odm_sepolicy.cil odm_file_contexts odm_property_contexts
chmod 0644 odm_sepolicy.cil odm_file_contexts odm_property_contexts
if [ -f /vendor/etc/selinux/precompiled_sepolicy ]; then
    mv /vendor/etc/selinux/precompiled_sepolicy /vendor/etc/selinux/precompiled_sepolicy.gpsd-binder-backup
fi
'
echo "Rebooting to load policy..."
reboot
root_remount

phase="installing the driver"
echo "Installing driver for $triple..."
ad push "$payload/android.hardware.gnss-service.gpsd-binder" /vendor/bin/hw/android.hardware.gnss-service.gpsd-binder >/dev/null
ad push "$payload/gpsd-binder-relay" /vendor/bin/gpsd-binder-relay >/dev/null
ad push "$payload/android/android.hardware.gnss-service.gpsd-binder.rc" /vendor/etc/init/ >/dev/null
ad push "$payload/android/android.hardware.gnss-service.gpsd-binder.xml" /vendor/etc/vintf/manifest/ >/dev/null
printf 'on early-boot\n    setprop vendor.gpsd.host %s\n    setprop vendor.gpsd.port %s\n' "$host" "$port" >"$work/gpsd-binder-emulator.rc"
ad push "$work/gpsd-binder-emulator.rc" /vendor/etc/init/ >/dev/null
ad shell sh <<'ANDROID'
set -eu
cd /vendor
mkdir -p -m 0755 etc/gpsd-binder-backup/init etc/gpsd-binder-backup/manifest
for file in etc/init/android.hardware.gnss*ranchu*.rc; do
    [ ! -f "$file" ] || mv "$file" etc/gpsd-binder-backup/init/
done
for file in etc/vintf/manifest/android.hardware.gnss*ranchu*.xml; do
    [ ! -f "$file" ] || mv "$file" etc/gpsd-binder-backup/manifest/
done
chown root:root bin/hw/android.hardware.gnss-service.gpsd-binder bin/gpsd-binder-relay etc/init/android.hardware.gnss-service.gpsd-binder.rc etc/init/gpsd-binder-emulator.rc etc/vintf/manifest/android.hardware.gnss-service.gpsd-binder.xml
chmod 0755 bin/hw/android.hardware.gnss-service.gpsd-binder bin/gpsd-binder-relay
chmod 0644 etc/init/android.hardware.gnss-service.gpsd-binder.rc etc/init/gpsd-binder-emulator.rc etc/vintf/manifest/android.hardware.gnss-service.gpsd-binder.xml
restorecon bin/hw/android.hardware.gnss-service.gpsd-binder bin/gpsd-binder-relay
ls -lZ bin/hw/android.hardware.gnss-service.gpsd-binder | grep -q hal_gnss_gpsd_binder_exec
ls -lZ bin/gpsd-binder-relay | grep -q gpsd_binder_relay_exec
ANDROID
if [ "$sdk" -ge 37 ]; then
    ad push "$payload/android/gpsd-binder-permissions.xml" /system/etc/permissions/gpsd-binder.xml >/dev/null
    ad shell 'chown root:root /system/etc/permissions/gpsd-binder.xml && chmod 0644 /system/etc/permissions/gpsd-binder.xml && restorecon /system/etc/permissions/gpsd-binder.xml'
fi
ad shell rm -rf /data/local/tmp/gpsd-binder
phase="rebooting and checking the driver"
echo "Rebooting to start the driver..."
reboot
ad shell service list | grep -F android.hardware.gnss.IGnss/default || fail "GNSS service did not register. Run: adb -s $serial logcat -d -s gpsd-binder gpsd-binder-relay"
ad shell ps -AZ | grep hal_gnss_gpsd_binder || fail "GNSS service is not running in its SELinux domain"
[ "$(ad shell getenforce | tr -d '\r')" = Enforcing ] || fail "SELinux is not enforcing"
echo "Installed. Using gpsd at $host:$port. Enable Android Location and open an app that requests location."
echo "Start this AVD with -writable-system -no-snapshot on future runs."
