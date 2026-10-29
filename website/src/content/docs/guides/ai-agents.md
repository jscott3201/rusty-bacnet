---
title: "Use rusty-bacnet with AI agents"
description: "Let an AI agent host such as Claude Code discover and read a BACnet site through a read-only MCP server."
---

`bacnet mcp serve` is a [Model Context Protocol](https://modelcontextprotocol.io) server: an agent host such as Claude Code or Codex starts it as a child process and calls its tools to discover devices, describe them, find and read their objects, read trend logs, event and audit logs and alarms, map the networks, decode frames, and follow points as they change. It is experimental in 0.15, built only with the CLI's `mcp` feature, and **read-only**: no configuration offers a tool that writes.

## Build it

The release executables leave the feature out. Build the CLI from a source checkout with it:

```sh
cargo install --path crates/bacnet-cli --locked --features mcp
```

## Authorize a profile

The server connects through a [TUI profile](/rusty-bacnet/tui/profiles/), whose `[mcp]` table limits what an agent may reach. An absent table reaches every network and device, at the default rates.

```toml
[mcp]
networks = [0, 2001]          # 0 is the local network
devices = ["1000-1999", "5"]
slow_networks = [2001]        # an MS/TP trunk behind a router: 2 requests a second
broad_discovery = false       # true allows a global or wide Who-Is, one a minute
```

Before an agent host can use the profile, a person consents in a terminal. The command shows what the table lets an agent reach and asks for the profile's name:

```sh
bacnet mcp authorize --profile plant-a
```

The authorization covers the table and the profile's transport settings. Widening the table later (another network or device, broad discovery, a higher rate or a larger cap) or changing the transport clears it, so the server refuses to start until it is given again. Narrowing the table doesn't.

A Who-Is is broad when it is global, has no range, or asks more than 1000 instances, and needs `broad_discovery`. A directed Who-Is without a range counts as broad too, unless its address is a device already discovered: an unknown address may be a subnet's broadcast address. When the table names `devices`, a Who-Is must also ask a range inside them.

## Add it to the agent host

Claude Code reads a project's `.mcp.json`. The host launches the command and speaks MCP on its stdin and stdout:

```json
{
  "mcpServers": {
    "bacnet": {
      "command": "bacnet",
      "args": ["mcp", "serve", "--stdio", "--profile", "plant-a"]
    }
  }
}
```

A live server needs an authorized profile, and refuses transport flags beside it: they would change the transport that was authorized. Without a profile only `--passive` runs: it builds no client and sends nothing, and offers only the discovery table, empty then, and the decoder. Logs go to stderr (`-v` or `RUST_LOG` for more), never to stdout, which carries only the protocol.

```sh
bacnet mcp serve --stdio --profile plant-a --passive
```

## What the agent can do

| Tool | What it does |
|---|---|
| `bacnet_discover` | Sends a Who-Is (local, to one network, to one address, or global) and lists the devices that answered |
| `bacnet_describe_device` | Summarises a device: vendor, model, firmware, protocol revision, supported services and object types, and object counts |
| `bacnet_list_objects` | Lists a device's objects, filtered by type, with their names on request |
| `bacnet_read` | Reads properties, by ReadPropertyMultiple with a ReadProperty fallback; `preset: "point"` reads what matters about a point |
| `bacnet_find_object` | Finds objects across the discovered devices by name, description, object or type, and says which devices it couldn't search fully |
| `bacnet_network_map` | Shows the networks the profile reaches, the routers to each and the devices on them; asking the routers again is a broad request |
| `bacnet_read_trend` | Reads a trend log's records, by count or time window, and summarises them: count, first and last time, minimum, maximum and mean, and counts of states |
| `bacnet_read_log_records` | Reads an Event Log's or Audit Log's records the same way, with event and audit notifications decoded, and counts them by event state, event type or audited operation |
| `bacnet_alarms` | Lists a device's objects in alarm or fault or waiting for acknowledgment, with states, times and priorities named, and optionally its alarm and enrollment summaries |
| `bacnet_decode` | Decodes a frame, NPDU or APDU given as hex into its layers and fields, without sending anything (also in passive mode) |
| `bacnet_watch` | Follows up to 32 points and, called again with its cursor, says which changed since the last call, for hosts without resource subscriptions |

Two prompts start common jobs: `survey_site` surveys a range of devices, their networks and their alarms, and `investigate_point` explains one point's value, status, priority array, alarms and trend. Every value comes as display text and a typed form with enumeration and bit names. A property a device can't give is an error row with the device's error class and code.

## Resources and subscriptions

The server also offers resources: `bacnet://devices` (paged: a long list names its next page, `bacnet://devices/after/{instance}`), `bacnet://device/{instance}`, a point as `bacnet://device/{instance}/object/{type}:{n}` (one property with `/property/{prop}`), alarms as `bacnet://alarms` and `bacnet://device/{instance}/alarms`, `bacnet://session` and `bacnet://ledger`. A host can subscribe to them, with `subscriptions/listen` on the 2026-07-28 revision of MCP or `resources/subscribe` on older ones such as 2025-06-18: the server watches each point with one COV subscription however many listeners share it, or polls it every 10 seconds when the device refuses COV, and tells the host when it changes. Reading a watched point sends nothing. Bursts of changes are folded into one update a second.

An agent can watch up to 32 points at once, and the server 256 in all. The subscription acknowledgment lists only the resources the server took on: a device not yet discovered, one the profile doesn't reach, or a point past the limit is left out (an older host's `resources/subscribe` gets an error for it instead). Subscriptions are always finite (five minutes, renewed) and are cancelled when the last listener leaves or the server stops; one the device can't be told about ends on its own. Hosts without subscriptions use `bacnet_watch` instead. In passive mode only the session resource and an empty ledger are offered.

## Safety

- **Device text is data.** Names, descriptions, vendor strings and character strings in a decoded frame come back in an `untrusted` envelope that says where each came from, with control characters, bidirectional overrides and other invisible format characters removed, links defanged and the length capped. A device can still say anything in them, so a model can still be misled by what it reads; the server's answer is that it can't change anything.
- **Limits hold on the server.** One request at a time per device, 20 a second in all, 20 a second per network (2 on a network marked slow, 5 on one marked fast), and 5 a second for the agent, with bursts of 20. Each result reports what it cost, including the time spent waiting, so an agent can slow down.
- **Results are paged.** Large lists, trend logs and alarm lists end with a signed cursor that only resumes the same call. A long object list read element by element goes on where the last call stopped. Each trend log request is charged, and a summary reads at most 20,000 records a call.
- **Reads are logged as summaries** on stderr, always: a line per call, and the counts per device when the server stops.
- **Site data reaches the agent's host.** Don't share an agent session that reads a sensitive site with tools that can send data elsewhere.

The design, including the write tools of a later phase, is in [`docs/design/mcp.md`](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/design/mcp.md).
