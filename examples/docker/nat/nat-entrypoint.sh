#!/bin/sh
# A NAT router for the NAT topology (#940, #942), on two networks: the
# "internet" side at NAT_PUBLIC and a site's private side at NAT_PRIVATE_NET.
#
# - Outbound: anything from the private side leaves with the public address
#   (masquerade). With NAT_FORWARD_TO set, the BBMD's own port keeps its
#   number too, so peers see it at NAT_PUBLIC:NAT_PORT, its global address.
# - Inbound, with NAT_FORWARD_TO set: UDP to NAT_PUBLIC:NAT_PORT is
#   port-forwarded to NAT_FORWARD_TO:NAT_PORT, the site's BBMD. Without it,
#   only replies to what the private side sent get in, as at a home router.
#
# Needs NET_ADMIN and net.ipv4.ip_forward=1.
set -eu

: "${NAT_PUBLIC:?}" "${NAT_PRIVATE_NET:?}"
NAT_PORT=${NAT_PORT:-47809}

if [ -n "${NAT_FORWARD_TO:-}" ]; then
  iptables -t nat -A PREROUTING -d "$NAT_PUBLIC" -p udp --dport "$NAT_PORT" \
    -j DNAT --to-destination "$NAT_FORWARD_TO:$NAT_PORT"
  iptables -t nat -A POSTROUTING -s "$NAT_FORWARD_TO" -p udp --sport "$NAT_PORT" \
    ! -d "$NAT_PRIVATE_NET" -j SNAT --to-source "$NAT_PUBLIC:$NAT_PORT"
fi
iptables -t nat -A POSTROUTING -s "$NAT_PRIVATE_NET" ! -d "$NAT_PRIVATE_NET" \
  -j SNAT --to-source "$NAT_PUBLIC"
echo "NAT router: public $NAT_PUBLIC, private $NAT_PRIVATE_NET, forward ${NAT_FORWARD_TO:-none}"
exec tail -f /dev/null
