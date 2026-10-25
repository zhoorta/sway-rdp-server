# Shared by the scripts behind `waydesk-admin access`, `cert` and `vpn` (sourced, not run). They run as root, started by waydesk-admin with fixed
# arguments; every value that came from the command line is checked here before it is used.
CONF=/etc/default/waydesk
STATE=/etc/waydesk

die() { echo "waydesk-admin: $*" >&2; exit 1; }
say() { printf '%s\n' "$*"; }
need_root() { [ "$(id -u)" -eq 0 ] || die "this needs root: run it with sudo"; }

# IPv4 dotted quad.
valid_ipv4() {
    local ip=$1 a b c d x
    [[ $ip =~ ^([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})$ ]] || return 1
    for x in "${BASH_REMATCH[@]:1}"; do [ "$x" -le 255 ] || return 1; done
}
# A DNS name (letters, digits, dots, hyphens).
valid_host() { [[ $1 =~ ^[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?(\.[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?)*$ ]] && [ ${#1} -le 253 ]; }
valid_port() { [[ $1 =~ ^[0-9]{1,5}$ ]] && [ "$1" -ge 1 ] && [ "$1" -le 65535 ]; }
# One entry of WAYDESK_ALLOW_FROM: a word, an address or a range (the daemon checks it again and refuses to start on a bad one).
valid_allow_item() {
    case $1 in
        any|private|loopback) return 0 ;;
    esac
    local addr=${1%%/*} prefix=""
    [[ $1 == */* ]] && prefix=${1#*/}
    if valid_ipv4 "$addr"; then
        [ -z "$prefix" ] || { [[ $prefix =~ ^[0-9]{1,2}$ ]] && [ "$prefix" -le 32 ]; }
    else
        [[ $addr =~ ^[0-9A-Fa-f:]+$ && $addr == *:* ]] || return 1
        [ -z "$prefix" ] || { [[ $prefix =~ ^[0-9]{1,3}$ ]] && [ "$prefix" -le 128 ]; }
    fi
}
valid_allow_list() { local i; [ -n "$1" ] || return 1; for i in ${1//,/ }; do valid_allow_item "$i" || return 1; done; }

ip_to_int() { local IFS=.; set -- $1; echo $(( ($1 << 24) + ($2 << 16) + ($3 << 8) + $4 )); }
int_to_ip() { echo "$(( ($1 >> 24) & 255 )).$(( ($1 >> 16) & 255 )).$(( ($1 >> 8) & 255 )).$(( $1 & 255 ))"; }
# The network address of ADDRESS/PREFIX, as ADDRESS/PREFIX.
network_of() {
    local n mask
    n=$(ip_to_int "$1"); mask=$(( (0xFFFFFFFF << (32 - $2)) & 0xFFFFFFFF ))
    echo "$(int_to_ip $(( n & mask )))/$2"
}
is_private_ipv4() {
    local n; n=$(ip_to_int "$1")
    # 10/8, 172.16/12, 192.168/16, 100.64/10 (Tailscale), link-local
    [ $(( n >> 24 )) -eq 10 ] || [ $(( n >> 20 )) -eq $(( 0xAC1 )) ] || [ $(( n >> 16 )) -eq $(( 0xC0A8 )) ] || [ $(( n >> 22 )) -eq $(( 0x191 )) ] || [ $(( n >> 16 )) -eq $(( 0xA9FE )) ]
}
# "ADDRESS/PREFIX IFACE" for every global IPv4 address of this machine.
# Interfaces made by containers, bridges and VPNs are not "the local network".
local_ipv4() { ip -4 -o addr show scope global 2>/dev/null | awk '$2 !~ /^(docker|br-|veth|virbr|lxc|lxd|cni|flannel|cali|wg|waydesk|tun|tap)/ {print $4, $2}'; }
# Every address of this machine, VPN and bridges included (for the names a certificate carries).
all_ipv4() { ip -4 -o addr show scope global 2>/dev/null | awk '$2 !~ /^(docker|br-|veth|virbr|lxc|lxd|cni|flannel|cali|tun|tap)/ {split($4,a,"/"); print a[1]}'; }

# The value of KEY in /etc/default/waydesk (the last assignment), empty when there is none.
conf_get() {
    [ -r "$CONF" ] || return 0
    sed -n "s/^$1=//p" "$CONF" | tail -n1 | sed -e 's/^"//' -e 's/"$//'
}
# Sets KEY="VALUE": the existing line is replaced, otherwise one is added at the end. Everything else in the file stays as it is.
conf_set() {
    local key=$1 value=$2 tmp
    tmp=$(mktemp "$CONF.XXXXXX")
    if grep -q "^$key=" "$CONF" 2>/dev/null; then
        K=$key V=$value awk 'BEGIN{k=ENVIRON["K"]; v=ENVIRON["V"]} index($0, k "=")==1 {print k "=\"" v "\""; next} {print}' "$CONF" > "$tmp"
    else
        { cat "$CONF" 2>/dev/null; printf '%s="%s"\n' "$key" "$value"; } > "$tmp"
    fi
    chmod 644 "$tmp"; mv "$tmp" "$CONF"
}
# Removes the assignments of KEY.
conf_unset() {
    local tmp; tmp=$(mktemp "$CONF.XXXXXX")
    grep -v "^$1=" "$CONF" > "$tmp" || true
    chmod 644 "$tmp"; mv "$tmp" "$CONF"
}

# Restarts the daemon; when it does not come up with the new settings the old file is put back and the reason is shown.
restart_daemon_or_restore() {
    local backup=$1
    systemctl restart waydesk.service 2>/dev/null || true
    sleep 2
    if ! systemctl is-active --quiet waydesk.service; then
        cp -p "$backup" "$CONF"
        systemctl restart waydesk.service 2>/dev/null || true
        echo "waydesk-admin: the daemon did not start with the new settings; the old ones are back. Why:" >&2
        journalctl -u waydesk.service -n 5 --no-pager 2>/dev/null | sed 's/^/    /' >&2
        exit 1
    fi
}
