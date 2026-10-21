# bacnet-cli

`bacnet` is the command-line tool of [Rusty BACnet](https://github.com/jscott3201/rusty-bacnet),
a BACnet protocol stack in Rust. It discovers devices, reads and writes
properties, subscribes to COV, handles alarms, files and routing, and decodes
captured traffic, as one-shot commands with JSON output, or in a full-screen
terminal UI (plain `bacnet` on a terminal, or `bacnet tui`).

## Install

```bash
cargo install bacnet-cli --locked --features sc-tls
```

The binary is called `bacnet`. Features:

- `tui` (default): the full-screen terminal UI. `--no-default-features`
  builds the one-shot commands without it.
- `sc-tls`: the BACnet/SC transport (WebSocket over TLS) and offline credential
  inspection (`bacnet sc-check`).
- `pcap`: `bacnet capture`, live or from a pcap file. It needs libpcap and its
  headers (`libpcap-dev` on Debian and Ubuntu).
- `serial`: MS/TP over a serial port (`--mstp`, `mstp-ports`,
  `tui --mstp-monitor`). The Linux release binaries include it.

Prebuilt binaries are attached to each
[release](https://github.com/jscott3201/rusty-bacnet/releases).

## Example

```bash
bacnet discover
bacnet read 192.168.1.100 ai:1 pv
bacnet --json readm 192.168.1.100 ai:1 pv,object-name
bacnet        # on a terminal: the TUI; create/select a B/IP, SC or MS/TP profile
bacnet tui --profile plant-a
bacnet tui --no-profile -i 127.0.0.1 -p 0 -b 127.0.0.255
```

Named B/IP, BACnet/SC and MS/TP profiles preview resolved settings, check the files
they name before anything is sent, and record an explicit first-use
authorization. SC profiles store certificate and key paths only, and keep the
device UUID they generate. Press `p` for profiles, `x` to disconnect and
`c` to reconnect. `BACNET_CONFIG_DIR` isolates both profiles and workspaces.
`--no-profile` or explicit network flags without `--profile` retain the
nonpersistent connection flow; B/IPv6 uses only that flow. The TUI is
read-only until `W` arms a time-boxed write session on a profile; every write
is previewed, kept in a revertible ledger and recorded in an audit log.
`--passive` transmits nothing.

One-shot commands refuse anything that changes a device unless
given `--allow-write`; communication control and reinitialize also need
`--yes` when stdin is not a terminal, or, at one, the device instance typed
(the target as given, for an address target). A refused command exits with
status 4.

The [CLI reference](https://github.com/jscott3201/rusty-bacnet/blob/main/docs/CLI.md)
lists every command and option.

With `sc-tls`, `bacnet sc-check --sc-cert node.pem` (new in 0.14.0) takes
optional `--sc-ca` and `--sc-key` files. It reports local certificate observations without a network connection;
exit 0 can include warnings and does not predict TLS acceptance. See the
[SC check guide](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/sc-check.md)
for human/JSON output and exit statuses.

## License

MIT
