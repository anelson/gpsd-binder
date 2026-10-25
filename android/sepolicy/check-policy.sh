#!/system/bin/sh
# Compile the installed split policy with neverallow checks, which init skips.
# Run as root before rebooting after a policy change.
set -eu

version=$(cat /vendor/etc/selinux/plat_sepolicy_vers.txt)
set -- /system/etc/selinux/plat_sepolicy.cil "/system/etc/selinux/mapping/$version.cil"
for file in "/system/etc/selinux/mapping/$version.compat.cil" \
    /system/etc/selinux/plat_sepolicy_genfs_*.cil \
    /system_ext/etc/selinux/system_ext_sepolicy.cil "/system_ext/etc/selinux/mapping/$version.cil" \
    /product/etc/selinux/product_sepolicy.cil "/product/etc/selinux/mapping/$version.cil" \
    /vendor/etc/selinux/plat_pub_versioned.cil /vendor/etc/selinux/vendor_sepolicy.cil \
    /odm/etc/selinux/odm_sepolicy.cil; do
    if [ -f "$file" ]; then
        set -- "$@" "$file"
    fi
done

policy=$(mktemp /data/local/tmp/gpsd-binder-policy.XXXXXX)
trap 'rm -f "$policy"' EXIT
secilc -m -M true -G -c "$(cat /sys/fs/selinux/policyvers)" -o "$policy" -f /dev/null "$@"
echo "SELinux policy checks passed (vendor policy $version)"
