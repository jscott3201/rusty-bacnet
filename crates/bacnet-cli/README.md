# bacnet-cli

`bacnet` is the command-line tool of [Rusty BACnet](https://github.com/jscott3201/rusty-bacnet),
a BACnet protocol stack in Rust. It discovers devices, reads and writes
properties, subscribes to COV, handles alarms, files and routing, and decodes
captured traffic, as one-shot commands with JSON output, or in a full-screen
terminal UI (plain `bacnet` on a terminal, or `bacnet tui`). With the `mcp`
feature, `bacnet mcp serve` offers the same reads, and writes a profile
allows, to an AI agent host.

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
- `mcp`: `bacnet mcp serve --stdio`, a Model Context Protocol server for AI
  agent hosts, and `bacnet mcp authorize`, which records a person's consent
  on a profile first. It brings `tui` along for the profiles. Experimental;
  the release binaries include it from 0.15.0.

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
bacnet mcp authorize --profile plant-a       # mcp feature: consent, then
bacnet mcp serve --stdio --profile plant-a   # what an agent host launches
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
`--passive` binds a receive-only B/IP UDP listener on the chosen interface and
port. The status bar says `listening` once the bind succeeds. Router
announcements populate Networks; `:find NAME` filters retained and newly overheard
I-Haves by exact name, without sending Who-Has. Names are learned from those
observations. Foreign-device settings are ignored: there is no registration,
renewal, responder or BACnet send path. Multicast, IPv6, SC and participant MS/TP
are explicitly unsupported in this mode; the separate MS/TP monitor still works.

This listener sees only UDP traffic delivered to this host. It cannot see unrelated
switched unicast traffic, and peers may never repeat an announcement. It is not
promiscuous packet capture or physical-network qualification. Startup router
announcements and unsolicited I-Haves follow Clauses 6.6.2/6.6.3.3 and 16.9/16.9.4.
The session keeps at most 4,096 recent I-Have observations/results and 4 MiB of
strings in each collection, refreshing repeated
station/object identities, plus at most 128 routers and 128 networks per router.
Oldest entries are evicted deterministically; names retain their existing LRU
limit. Disconnect, replacement and UI closure release the owned listener.

One-shot commands refuse anything that changes a device unless
given `--allow-write`; communication control and reinitialize also need
`--yes` when stdin is not a terminal, or, at one, the device instance typed
(the target as given, for an address target). A refused command exits with
status 4.

The [CLI reference](https://github.com/jscott3201/rusty-bacnet/blob/main/docs/CLI.md)
lists every command and option.

With `sc-tls`, `bacnet sc-check --sc-cert node.pem` (since 0.14.0) takes
optional `--sc-ca` and `--sc-key` files. It reports local certificate observations without a network connection;
exit 0 can include warnings and does not predict TLS acceptance. See the
[SC check guide](https://github.com/jscott3201/rusty-bacnet/blob/main/docs/sc-check.md)
for human/JSON output and exit statuses.

## License

MIT
