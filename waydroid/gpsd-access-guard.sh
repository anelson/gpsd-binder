#!/system/bin/sh
# gpsd-access-guard.sh - let only uid 0 open TCP connections to host gpsd.
#
# The GNSS driver (gpsd-binder) runs as uid 0. Without this rule every
# Android process, including apps that have no location permission, could
# open 192.168.240.1:2947 and read raw positions or send commands to gpsd.
# The rule answers such a connection with a TCP reset unless the socket
# belongs to uid 0.
#
# Where the rule is kept (IPv4 filter table):
#   chain gpsd_guard          netd does not know this chain and never flushes it.
#   one jump at the top of    netd shares OUTPUT with vendor code: when netd
#   OUTPUT                    starts it adds its own jumps but keeps rules that
#                             others added. netd flushes its own child chains,
#                             including oem_out, each time it starts, so the
#                             rule is not kept in oem_out.
#
# Guarded destinations: 192.168.240.1:2947 always, and also
# service.gpsd.host:service.gpsd.port when those properties name another IPv4
# address or port (for example a test port such as 2948). The default is
# guarded always because the first run, at post-fs-data, comes before the
# early-boot trigger that sets the properties.
#
# Why "--uid-owner 1-4294967294" and not "! --uid-owner 0": after a socket is
# closed, the kernel still sends packets for it (the final ACK, or a reset),
# and those packets have no owner. A negated match catches packets with no
# owner, so it reset the forwarder's end of every closed HAL connection and
# left that end retransmitting for minutes. A positive range never matches a
# packet with no owner, and still matches every socket owned by uid 1 or more.
#
# The script is idempotent. It replaces the chain in one iptables-restore
# transaction, adds the jump only when it is missing, and holds a lock so that
# runs started at the same moment do not interfere.
#
# init runs it from /vendor/etc/init/gpsd-binder-waydroid.rc
# once at post-fs-data (before zygote starts any app), again after boot,
# whenever netd starts again, and whenever the host or port property changes.
# Manual use, as root:  sh /vendor/bin/gpsd-access-guard.sh
# Result lines go to logcat with the tag gpsd-guard.

PATH=/system/bin:/vendor/bin
CHAIN=gpsd_guard
TAG=gpsd-guard
WHY=${1:-manual}

say() { log -t "$TAG" -p "$1" "$2"; echo "$2"; }

rule() {
    echo "-A $CHAIN -d $1/32 -p tcp -m tcp --dport $2 -m owner --uid-owner 1-4294967294 -j REJECT --reject-with tcp-reset"
}
is_ipv4() {
    case "$1" in
        *[!0-9.]*|.*|*.|*..*) return 1 ;;
        *.*.*.*.*) return 1 ;;
        *.*.*.*) ;;
        *) return 1 ;;
    esac
    for o in $(echo "$1" | tr . ' '); do
        [ "$o" -le 255 ] || return 1
    done
    return 0
}
is_port() {
    case "$1" in ''|*[!0-9]*) return 1 ;; esac
    [ "$1" -ge 1 ] && [ "$1" -le 65535 ]
}

apply() {
    host=$(getprop service.gpsd.host)
    port=$(getprop service.gpsd.port)
    h=${host:-192.168.240.1}
    p=${port:-2947}

    rules=$(rule 192.168.240.1 2947)
    guarded="192.168.240.1:2947"
    if is_ipv4 "$h" && is_port "$p"; then
        if [ "$h:$p" != "192.168.240.1:2947" ]; then
            rules="$rules
$(rule "$h" "$p")"
            guarded="$guarded and $h:$p"
        fi
    else
        say w "($WHY) service.gpsd.host/port '$host:$port' is not an IPv4 address and port; only 192.168.240.1:2947 is guarded"
    fi

    if ! printf '*filter\n:%s - [0:0]\n%s\nCOMMIT\n' "$CHAIN" "$rules" | iptables-restore -w 10 --noflush; then
        say e "($WHY) could not load chain $CHAIN; gpsd is NOT protected (are the host modules ip_tables, iptable_filter, xt_owner, ipt_REJECT loaded?)"
        exit 1
    fi
    if ! iptables -w 10 -C OUTPUT -j "$CHAIN" 2>/dev/null; then
        if ! iptables -w 10 -I OUTPUT 1 -j "$CHAIN"; then
            say e "($WHY) could not add the jump to $CHAIN in OUTPUT; gpsd is NOT protected"
            exit 1
        fi
    fi
    say i "($WHY) $CHAIN active: TCP to $guarded is reset for every uid except 0"
    return 0
}

# mksh marks descriptors that "exec" opens above 2 as close-on-exec, so the
# lock descriptor is opened for this block instead; flock (toybox) inherits it.
{
    flock 9
    apply
} 9>/dev/gpsd_guard.lock
