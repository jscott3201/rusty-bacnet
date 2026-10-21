#!/bin/sh
# Add the static IP routes in BACNET_ROUTES, then run the command.
#
# BACNET_ROUTES holds `ip route` arguments separated by semicolons, for
# example "172.20.4.0/24 via 172.20.0.3". Docker doesn't route IP between its
# bridge networks, so the foreign-device scenario's hosts reach each other
# through the ip-router service with these routes. Adding a route needs the
# NET_ADMIN capability.
set -eu

old_ifs=$IFS
IFS=';'
for route in ${BACNET_ROUTES:-}; do
  IFS=$old_ifs
  # Word splitting of $route into ip's arguments is intended.
  # shellcheck disable=SC2086
  ip route replace $route
  IFS=';'
done
IFS=$old_ifs

exec "$@"
