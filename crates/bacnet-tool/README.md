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
- `decode::tree`: frames, NPDUs and APDUs as trees of fields with their
  octet ranges, with no traffic. Header summaries are in `bacnet_ops::decode`.
- `bbmd`: B/IP broadcast management tables as plain data, the BDT and FDT
  reads through the `bacnet-ops` client and its limits, the survey that
  follows each BDT to the BBMDs it lists, and the consistency matrix over
  them (one-way entries, missing own entries, mismatched masks).

It is a hosted crate: it needs `std` and Tokio, and has no terminal or MCP
dependency. Shared decode-test builders use `bacnet-ops/test-support`;
the tool's own `test-support` feature retains its filesystem fault hooks.

The API follows the needs of the `bacnet` tool and changes with it before
1.0.
