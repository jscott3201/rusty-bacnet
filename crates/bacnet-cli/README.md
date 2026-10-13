# bacnet-cli

`bacnet` is the command-line tool of [Rusty BACnet](https://github.com/jscott3201/rusty-bacnet),
a BACnet protocol stack in Rust. It discovers devices, reads and writes
properties, subscribes to COV, handles alarms, files and routing, and decodes
captured traffic, as one-shot commands, in an interactive shell, or in a
full-screen terminal UI (`bacnet tui`).

## Install

```bash
cargo install bacnet-cli --locked --features sc-tls
```

The binary is called `bacnet`. Features:

- `tui`: `bacnet tui`, the full-screen terminal UI. It is off by default, and
  out of the release binaries, until the terminal UI ships in 0.14.0.
- `sc-tls`: the BACnet/SC transport (WebSocket over TLS).
- `pcap`: `bacnet capture`, live or from a pcap file. It needs libpcap and its
  headers (`libpcap-dev` on Debian and Ubuntu).

Prebuilt binaries are attached to each
[release](https://github.com/jscott3201/rusty-bacnet/releases).

## Example

```bash
bacnet discover
bacnet read 192.168.1.100 ai:1 pv
bacnet --json readm 192.168.1.100 ai:1 pv,object-name
bacnet tui    # built with --features tui
```

The [CLI reference](https://github.com/jscott3201/rusty-bacnet/blob/main/docs/CLI.md)
lists every command and option.

## License

MIT
