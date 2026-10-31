---
title: "Use rusty-bacnet with AI agents"
description: "Let an AI agent host such as Claude Code discover and read a BACnet site through an MCP server, and write only with a person's approval."
---

`bacnet mcp serve` is a [Model Context Protocol](https://modelcontextprotocol.io) server: an agent host such as Claude Code or Codex starts it as a child process and calls its tools to discover devices, describe them, find and read their objects, read trend logs, event and audit logs and alarms, map the networks, decode frames, and follow points as they change. It is **experimental**, ships in the prebuilt `bacnet` downloads from 0.15.0, and is **read-only by default**: a tool that writes is offered only when the profile opts in, and every write then needs a person's approval: in the host's confirmation form for a low-priority (T1) change, or ahead of time in a plan grant the person authorizes with `bacnet mcp authorize`, which covers the others it names (see [Writes](#writes)).

## Install it

Every prebuilt `bacnet` download on the [installation page](/rusty-bacnet/start/installation/) includes `bacnet mcp serve`. A source build enables it with the CLI's `mcp` feature:

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
mcp_writes = "never"          # the default; "elicit" offers bacnet_write and bacnet_revert
```

The table also sets the rates (`max_rate`, `network_rate`, `slow_rate`, `fast_rate`, `principal_rate`, `principal_burst`) and the per-call caps (`max_values`, `page_devices`, `page_objects`, `max_text`, `call_budget_s`). The [profile key reference](https://github.com/jscott3201/rusty-bacnet/blob/v0.15.0/docs/mcp.md#profile-keys) lists every key with its meaning, default and limits; a value outside the limits stops the server from starting.

Before an agent host can use the profile, a person consents in a terminal. The command shows what the table lets an agent reach and asks for the profile's name:

```sh
bacnet mcp authorize --profile plant-a
```

The authorization covers the table and the profile's transport settings. Widening the table later (another network or device, broad discovery, a higher rate or a larger cap, a network no longer marked slow or fast, or turning on writes or arming), adding or changing a plan in any way, or changing the transport clears it, so the server refuses to start until it is given again. Narrowing the table's reach doesn't; narrowing a plan does.

A Who-Is is broad when it is global, has no range, or asks more than 1000 instances, and needs `broad_discovery`. A directed Who-Is without a range counts as broad too, unless its address is a device already discovered: an unknown address may be a subnet's broadcast address. When the table names `devices`, a Who-Is must also ask a range inside them, and when it names `networks`, a global Who-Is is refused.

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
| `bacnet_network_map` | Shows the networks the profile reaches, the routers to each and the devices on them; asking the routers again is a broad request. On B/IP, `bbmd: true` also reads the BBMD tables in reach and checks that they agree (needs `broad_discovery`) |
| `bacnet_read_trend` | Reads a trend log's records, by count or time window, and summarises them: count, first and last time, minimum, maximum and mean, and counts of states |
| `bacnet_read_log_records` | Reads an Event Log's or Audit Log's records the same way, with event and audit notifications decoded, and counts them by event state, event type or audited operation |
| `bacnet_alarms` | Lists a device's objects in alarm or fault or waiting for acknowledgment, with states, times and priorities named, and optionally its alarm and enrollment summaries |
| `bacnet_decode` | Decodes a frame, NPDU or APDU given as hex into its layers and fields, without sending anything (also in passive mode) |
| `bacnet_watch` | Follows up to 32 points and, called again with its cursor, says which changed since the last call, for hosts without resource subscriptions |

Two prompts start common jobs: `survey_site` surveys a range of devices, their networks and their alarms, and `investigate_point` explains one point's value, status, priority array, alarms and trend. Every value comes as display text and a typed form with enumeration and bit names. A property a device can't give is an error row with the device's error class and code.

## Writes

With `mcp_writes = "elicit"` in the `[mcp]` table the server also offers `bacnet_write` and `bacnet_revert`. They use the TUI's write checks: the same preview tiers, the same [ledger and audit log](/rusty-bacnet/tui/writes/) in the profile's workspace, so the TUI lists an agent's changes (marked `[mcp: user]`) and can revert them.

- **A person approves each write.** The first call reads the target and previews the change; the agent host then shows a confirmation form (the value now and after, who holds the slot), and only a confirmed retry sends it. A confirmation is good once, for that agent, for two minutes, and is refused if the value changed in between.
- **The host's form approves low-priority writes only.** The server can't check that a person answered the host's form, so it accepts it for priority 9 to 16, or a property that isn't commandable. Priorities 1 to 8, Out_Of_Service, Relinquish_Default, batches and relinquishing another commander's slot are refused (`needs-verified-approver`), and priority 6 always. The form needs a host on the 2026-07-28 revision of MCP that takes form elicitations; other hosts are refused (`elicitation-unsupported`) and write only through a plan grant. One call carries at most 20 writes.
- **Arming.** As in the TUI, writes need an armed session. With `mcp_arm = "elicit"` the first confirmation also arms one for 15 minutes; otherwise only a plan grant arms MCP writes.
- **Plan grants.** An `[mcp.plan]` table names devices, properties, a priority range, a number of writes and how many hours (8 at most) after `bacnet mcp authorize` they may run without a prompt. Authorizing says so in capitals; a write outside the plan is refused. The count of writes is kept in the profile's workspace, so restarting the server doesn't reset it; authorizing again starts a fresh count. A plan, like `mcp_arm = "elicit"`, needs `mcp_writes = "elicit"`:

  ```toml
  # A plan grant: writes approved in advance
  [mcp]
  mcp_writes = "elicit"
  mcp_arm = "elicit"            # the first confirmation arms a session

  [mcp.plan]
  devices = ["1201"]            # required
  properties = ["present-value"] # the default
  priority_min = 8              # default 8; priority_max defaults to 16
  max_writes = 50               # required, 1 to 10,000
  hours = 4                     # required, 1 to 8
  reason = "night setback test" # optional
  ```
- **Targets are numbers.** The device instance and `type:instance` objects come from the agent, never from a name a device reported.
- **Not offered:** DeviceCommunicationControl, ReinitializeDevice, or anything that creates or deletes objects.

`bacnet_revert` puts back what this agent changed: a slot is relinquished (or given back the value another commander had), another property gets its old value. While the TUI has the profile open it holds the ledger, so a write is refused (`workspace-locked`) until it closes; reads go on.

## Resources and subscriptions

The server also offers resources: `bacnet://devices` (paged: a long list names its next page, `bacnet://devices/after/{instance}`), `bacnet://device/{instance}`, a point as `bacnet://device/{instance}/object/{type}:{n}` (one property with `/property/{prop}`), alarms as `bacnet://alarms` and `bacnet://device/{instance}/alarms`, `bacnet://session` and `bacnet://ledger`. A host can subscribe to them, with `subscriptions/listen` on the 2026-07-28 revision of MCP or `resources/subscribe` on older ones such as 2025-06-18: the server watches each point with one COV subscription however many listeners share it, or polls it every 10 seconds when the device refuses COV, and tells the host when it changes. Reading a watched point sends nothing. Bursts of changes are folded into one update a second.

An agent can watch up to 32 points at once, and the server 256 in all. The subscription acknowledgment lists only the resources the server took on: a device not yet discovered, one the profile doesn't reach, or a point past the limit is left out (an older host's `resources/subscribe` gets an error for it instead). Subscriptions are always finite (five minutes, renewed) and are cancelled when the last listener leaves or the server stops; one the device can't be told about ends on its own. Hosts without subscriptions use `bacnet_watch` instead. `bacnet://ledger` lists this agent's changes still in effect. In passive mode only the session resource and the ledger are offered.

## Safety

- **Device text is data.** Names, descriptions, vendor strings and character strings in a decoded frame come back in an `untrusted` envelope that says where each came from, with control characters, bidirectional overrides and other invisible format characters removed, links defanged and the length capped. A device can still say anything in them, so a model can still be misled by what it reads; the server's answer is that nothing changes without a person's approval.
- **Limits hold on the server.** One request at a time per device, 20 a second in all, 20 a second per network (2 on a network marked slow, 5 on one marked fast), and 5 a second for the agent, with bursts of 20. Each result reports what it cost, including the time spent waiting, so an agent can slow down.
- **Results are paged.** Large lists, trend logs and alarm lists end with a signed cursor that only resumes the same call. A long object list read element by element goes on where the last call stopped. Each trend log request is charged, and a summary reads at most 20,000 records a call.
- **Reads are logged as summaries** on stderr, always: a line per call, and the counts per device when the server stops.
- **Site data reaches the agent's host.** Don't share an agent session that reads a sensitive site with tools that can send data elsewhere.

The design, with the approval channels of later releases, is in [`docs/design/mcp.md`](https://github.com/jscott3201/rusty-bacnet/blob/v0.15.0/docs/design/mcp.md).
