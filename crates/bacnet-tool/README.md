# bacnet-tool

The `bacnet` tool's own layer over
[`bacnet-ops`](https://crates.io/crates/bacnet-ops) in the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. The
one-shot commands, the terminal UI and the MCP server (`bacnet-mcp`) share
it; an application that reads and writes devices through `bacnet-ops`
doesn't need it.

- `safety`: the write-safety core the terminal UI and the MCP server
  share: session levels, confirmation tiers, the write preview, the
  crash-safe ledger of pre-images, the redacted append-only audit log,
  the workspace lock, and the client half of a write (read again, refuse a
  stale preview, record, send, settle, revert).
- `class`: what each command does to a device (read, write or device
  management), the table the front ends' safety levels share.
- `decode`: frames of the tapped links (B/IP, B/IPv6, BACnet/SC, MS/TP),
  NPDUs and APDUs as one-line summaries and trees of fields with their
  octet ranges, with no traffic.
- `bbmd`: B/IP broadcast management tables as plain data, the BDT and FDT
  reads through the `bacnet-ops` client and its limits, the survey that
  follows each BDT to the BBMDs it lists, and the consistency matrix over
  them (one-way entries, missing own entries, mismatched masks).

It is a hosted crate: it needs `std` and Tokio, and has no terminal or MCP
dependency. Its `test-support` feature adds frame builders for the CLI's
decode tests and is not a stable API.

The API follows the needs of the `bacnet` tool and changes with it before
1.0.
