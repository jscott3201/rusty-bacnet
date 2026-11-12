# MCP server (`bacnet-mcp`, `bacnet mcp serve`)

A [Model Context Protocol](https://modelcontextprotocol.io) server for
BACnet sites, read-only unless a profile offers writes: phase 1 (#2066, the
tools), phase 2 (#2067, resources and subscriptions) and phase 3 (#2068,
writes) of the MCP epic (#2064), and most of phase 4 (#2185, #2201,
[attached to the TUI](#attached-to-the-tui), with writes the TUI
approves). The design, with
the later phases, is [design/mcp.md](design/mcp.md); the user guide is the
website's [AI agents](https://jscott3201.github.io/rusty-bacnet/guides/ai-agents/)
page. Experimental in 0.15.

## Pieces

| Piece | Where | What it does |
|---|---|---|
| `bacnet-mcp` | `crates/bacnet-mcp` | `McpServer<T>`: the tool surface, prompts, policy, untrusted-text envelope, signed cursors. Out of `default-members`; publishable, because the CLI depends on it. Default feature `stdio`. |
| Engine extensions | `bacnet-ops` | `RequestLimits::with_network_rates`, `turn_on` and `slot`; per-caller `Account` and `TokenBucket` through `limits::charged`; `Engine::who_is`, `discover_routers`, `devices`, `device`, `address_style`; `read_trend` charged per request; `alarm_summary` and `enrollment_summary`; `inventory_device` and `inventory_object`, the 223 export's reads a page at a time (#2436) |
| Decoder | `bacnet-ops::decode` | The capture screen's frame summary and field tree, moved from the CLI for `bacnet_decode`: any tapped link (B/IP, B/IPv6, BACnet/SC, MS/TP), or a bare NPDU or APDU; a field quoting a character string is marked as device text |
| Watch hub | `bacnet-ops::watch` | `Hub`: one managed COV subscription or poller per point, shared by every listener, holding the latest values (phase 2); `Engine::device_events` |
| `bacnet mcp` | `crates/bacnet-cli/src/mcp.rs` | `serve --stdio [--profile NAME] [--passive]`, `connect [--profile NAME]` (#2185) and `authorize --profile NAME`, behind the CLI feature `mcp` (which brings `tui`, for the profiles; the release binaries include it from 0.15.0, and a source build enables it with `--features mcp`). Classified `Read` in `bacnet_ops::class`. |
| `[mcp]` profile table | `crates/bacnet-cli/src/tui/profile/mcp.rs` | networks, devices, slow and fast networks, broad discovery, rates and caps; `mcp_writes`, `mcp_arm` and `[mcp.plan]` (#2068) |
| Write safety core | `bacnet-ops::safety` | The TUI's plan, ledger, audit log and workspace lock, and the engine half of a write (`desk`: pre-image reads, the stale check, the ledger and audit around each send, revert), shared by the TUI and MCP (#2068) |
| Writes | `crates/bacnet-mcp/src/writes/`, `tools/write.rs`, `tools/revert.rs` | The write session, the approvers (`ClientElicitation`, `PlanApprover`, `Deny`), the sealed confirmation state, and the two write tools |
| Attached mode | `crates/bacnet-mcp/src/attach/`, `crates/bacnet-cli/src/tui/attach.rs` | The TUI's local socket or, on Windows, named pipe (`attach::Listener`), the peer checks, and `bacnet mcp connect`'s bridge (`attach::bridge`) (#2185, #2201) |

The SDK is `rmcp` 3.9.0 (pinned, server role, stdio only). It answers both
the 2026-07-28 request form (`server/discover`, version and capabilities in
each request's `_meta`) and the older `initialize` handshake.

## Tools

The read tools are read-only (`readOnlyHint: true`, `destructiveHint:
false`); the two [write tools](#writes) are listed only when the profile
offers writes. All have input and output schemas and structured content
plus a text rendering. An `object` argument is `type:instance` (`ai:1`,
`analog-output:3`) or, as the coming BACnet RDF encoding writes one,
`type,instance` (`analog-output,3`); a log tool's `from` and `to` also take
that encoding's date-time string, `2026-11-10 2 16:35:00.00`. Every form is
in the CLI reference's [Input Forms](CLI.md#input-forms).

| Tool | Notes |
|---|---|
| `bacnet_discover` | Who-Is (local, network, directed, global), then the discovery table, paged (pages index the live table, which can change between them). Broad (global, no range, or more than 1000 instances; directed without a range to an address that isn't a known device) needs `broad_discovery`, one a minute, whatever the reach; with `[mcp] devices` the range must also lie inside them. A send the transport refuses isn't charged. A network-scope Who-Is takes that network's slot. The one tool in passive mode, where it only lists (by default) and refuses to send. |
| `bacnet_read` | RPM with the engine's RP fallback; array elements by RP; `preset` `point` or `device`; at most `max_values` rows a call, then a cursor |
| `bacnet_list_objects` | Object_List, cached five minutes per device for paging; `types` filter; `names` reads each name (50 a page). A list read element by element stops at the call's budget and keeps its place, so the next call (with `next_cursor`) goes on from there; `list.complete` says when it's done. Calls on one device take turns, so progress never goes back. A resumed read checks the count again and starts over if it changed, and one that hasn't gone on for 10 minutes starts over; a list read element by element is kept five minutes from when it completed; an element the device can't give is a gap (`list.gaps`); at most 64 devices' lists are kept |
| `bacnet_describe_device` | The Device object's summary properties, services and object types by name, the address and route; `count_objects` |
| `bacnet_find_object` | Objects across the discovery table, within reach and an optional device `range`: by `name` or `description` (a case-insensitive substring of the cleaned device text), `object` or `types`. Lists come from `bacnet_list_objects`' cache (read once, kept five minutes); a text match reads each candidate's name (and description, if asked) once, kept five minutes too. A page holds up to 100 matches (or the policy's object page), and a full page ends before the next device is read; the cursor packs a device and a candidate index. Names are kept cleaned and capped at 4,096 characters, which a search matches against (a result shows the policy's text cap of them), at most 4,096 objects a device and 16,384 in all. A device whose list read stopped is under `incomplete` with why, after searching what was read; one that failed is under `skipped` with its error; a list read cut by the budget resumes on the next page. A device that stops answering while its names are read is under `incomplete` too, and the search moves on without a cursor back to it, so a silent device doesn't cost each later page a timeout; search it alone to retry. |
| `bacnet_network_map` | Networks in `[mcp] networks`, each with its routers (from the last Who-Is-Router-To-Network and from the routed devices' router addresses), its speed and rate under the policy, and its devices in reach; and the routers with the networks each reaches. `send: true` broadcasts a Who-Is-Router-To-Network: it needs `broad_discovery`, goes at most once a minute (checked and claimed under one lock), takes the local network's slot and is charged. The wait for answers ends with the call's budget, counted after the wait for the rate slot. A query cancelled while it waits (for the slot or for answers) keeps its claim on the minute but records no routers; at most 256 routers are kept, replaced by each finished query (one asking about a single network merges into them, the routers just heard first). `bbmd: true` adds the BBMD tables and a consistency verdict (see [BBMD tables](#bbmd-tables)). No network-layer writes, table writes or registrations. |
| `bacnet_read_trend` | A Trend Log or Trend Log Multiple through `Engine::read_trend`, each request charged. From the oldest record or after `from`, up to `to`; at most `max_records` (500 by default, 2,000 at most, and about 60 kB of structured content) a call, then a cursor holding the log's next sequence number (a log that has more but gives no sequence number to resume from returns `truncated` without a cursor, and says why under `stopped`). `from` after `to` is a JSON-RPC -32602.
 Records: time, sequence number, `values` as `Datum`s (a failure as its error), status flags named, `flagged` when a status flag is set or the record is a log status change. A record under a tag its CHOICE doesn't assign (a later revision's, #2474) has `kind` `unknown-choice` and its `choice`, an `unknown-choice` element (its tag, whether constructed, and its octets as hex), and is flagged; a Trend Log Multiple member under one is that element in `values`. A summary of the records read comes with every call: count, flagged, log status and time-change counts, first and last time, and per column (a member of a Trend Log Multiple) count, min, max and mean of numbers, counts of booleans and enumerations by value, failures. Once the device has answered the first page, a call reads the log's Log_DeviceObjectProperty once (one more charged request, through `Engine::log_columns`, so a silent device still costs one timeout) and names each column's enumerations and bit strings by the property it samples, in `values` and in the state counts (#2091); when that read fails or names nothing, values keep their numbers. State keys are therefore names when the reference names them and numbers otherwise, and can differ between calls on the same log if the reference read fails on one of them. A column that samples a binary or multi-state object's state (its Present_Value and the like) counts its samples as states, keyed `active`/`inactive` or by state number, never as numbers to average; when the reference read fails, the call doesn't know the column holds states, so a multi-state column falls back to `numeric` and `numeric` versus `states` can also differ between calls. The call then reads that object's Active_Text and Inactive_Text, or State_Text: one more charged request an object, once a call, for at most 8 objects on the log's own device, and skipped once the call's budget is spent (#2122). The texts are device strings, so they never become keys or join a value: the column's `state_texts` maps each state key it counted to its text, as the device sent it, in an untrusted envelope (cleaned and capped there, with `truncated` set when cut) (source `device N multi-state-value:1 state-text`, `active-text` or `inactive-text`), and is null when no key has one (a refused read, an empty text, a state past the end of State_Text). `summary: true` reads up to 20,000 records and returns only the summary. `pages`, `more_items` (answers cut short by the device) and `gaps` report how the reading went. |
| `bacnet_read_log_records` | An Event Log or Audit Log (#2103), read and paged as `bacnet_read_trend` reads a trend log: the same `from`, `to`, `max_records`, `summary` and cursor arguments, the same limits and budget, each request charged, and the same `pages`, `more_items`, `gaps`, `done` and `stopped`. A sibling tool rather than more of the trend tool: these records are notifications, not samples, so they have no columns to name or average. Records: time, sequence number (an Audit Log counts in 64 bits), `kind` and `flagged` (a log status change, an event into a state other than normal, a failed operation, or a record of either log under a tag its CHOICE doesn't assign, whose `kind` is `unknown-choice` with the element in `choice`, #2474, #2483). An event notification (`notification`) comes through `bacnet_ops::format::log_records`: the initiating device and event object, timestamp, class, priority, event type, notify type and states named (an ack-notification has no `from_state` or `ack_required`), and its event values through the shared notification renderer (#2073). An audit notification (`audit`) names the operation and the target property, gives the source and target devices and objects, decodes the values written and held as the target property's, and names a failure's error class and code. The message and the comments are device text, in untrusted envelopes (source `device N event-log:1 log-buffer`). The summary counts records by kind, then for an Event Log the notifications by the state entered (`by_to_state`), event type and notify type, and for an Audit Log the operations by name (`by_operation`) and how many failed. Any other object is refused (`invalid-argument` on `object`). |
| `bacnet_alarms` | GetEventInformation through `bacnet_ops::format::alarms` (#2077): objects, states, transitions, notify types, timestamps and priorities named; paged over the device's answers (`more_events`) up to `max` (100) a call, the cursor holding the last object; once the call's budget is spent no further answer is asked for, and what was read comes back with `more_events` and a cursor (`format::alarms::read_event_information_until`).  `summaries: true` adds GetAlarmSummary and GetEnrollmentSummary on the first page; a device refusing one reports it under `summary_errors`. When the events spent the call's budget, neither is sent, and each shows under `summary_errors` as `{"skipped": "budget"}`. Every datum is passed through the envelope check. |
| `bacnet_watch` | Points on the watch hub, and what changed since the last call (see [the tool-only fallback](#the-tool-only-fallback)). `idempotentHint: false`. Not listed in passive mode. |
| `bacnet_decode` | Hex (separators and `0x` ignored, at most 6,144 octets, room for the largest BACnet/SC message the stack accepts) as `bip`, `bip6`, `mstp`, `sc`, `npdu`, `apdu` or `auto` (by the first octets; SC must be named). Returns a one-line `text`, the decoder's `summary` and the field `tree` (labels with `[start, end)` octet ranges, at most 600 fields). A label quoting a character string is an envelope with source `decoded frame`; other labels are cleaned, and a long one (an unknown tag's octets) is cut at 512 characters with `truncated: true`. No traffic: listed in passive mode too. |
| `bacnet_export_223` | Experimental (#2436): one device's analog, binary and multi-state objects and integer values as a proposed Standard 223 point list in Turtle, the CLI's `bacnet export 223-experimental` through the engine, paged with a cursor. Each page's Turtle is a second text block; the structured content summarises the page. See [Standard 223 export](#standard-223-export-experimental). |

Prompts: `survey_site` (range, network) and `investigate_point` (device,
object). Their arguments are checked against the target grammar. The survey
also maps the networks and lists each device's alarms; the point
investigation also checks its alarms and summarises a trend log of it.

## Standard 223 export (experimental)

`bacnet_export_223` (#2436) writes what `bacnet export 223-experimental`
writes, from the same code (`bacnet_ops::export::s223`): one 223 property
for each analog, binary and multi-state input, output and value and each
integer value, with a BACnet external reference to its Present_Value. Its
title says experimental, as the CLI command's name does: the draft standard
and the mapping can change between releases. Read-only; not listed in
passive mode.

- **Arguments.** `device` as the other tools take it; `base_iri` (default
  `http://example.org/bacnet/`, a documentation domain), `types` (any of the
  ten types, by the target grammar's names: `ai`, `binary-output`,
  `integer-value` and so on; left out, all ten) and `write_priority` (1 to
  16). The export's own checks refuse a bad base IRI, a priority outside 1
  to 16, a type it doesn't map or an empty `types`, with its own messages,
  as `invalid-argument` before anything is sent.
- **Base IRI.** Each device's nodes go under `<base_iri>device-<instance>/`
  and are named from object identifiers. A device instance is unique only
  within one BACnet internetwork, so give each site a base of its own, or
  two sites' models merged into one graph would share node names.
- **Pages.** A call returns one page. It takes the device's Object_List
  as `bacnet_list_objects` does (from its five-minute cache, or read
  element by element across calls, each going on where the last stopped;
  while it is incomplete a page holds no object and its cursor goes on
  reading the list), reads the device's name, then reads the objects of
  the selected types in Object_List order, one at a time from the cursor's
  place, through `Engine::inventory_object`. A page's Turtle stays under
  24,000 bytes: the page ends before an object likely to pass that (judged
  by its largest point so far), and an object that passes it anyway goes
  back to the next page, where it is read again. A point larger than that
  on its own goes out on a page alone, flagged `oversized`. A page ends too
  at 50 objects (or the policy's object page, if smaller), or once the
  call's budget is spent; with objects left, it takes at least one. Then
  `next_cursor` points at the first object not on the page: nothing past
  it is read but, now and then, that one. The cursor is bound, as every
  cursor is, to the call's other arguments (device, `types`, `base_iri`,
  `write_priority`), the principal and ten minutes; another call's cursor
  is refused (`invalid-argument` on `cursor`) before anything is sent.
- **The cursor holds the list too**: a fingerprint of the matching objects
  in order. When the Object_List has changed since the cursor's page
  (objects added, removed or moved; the list is read again once its five
  minutes in the cache are up), the call fails (`failed`) after the list
  read, with nothing more sent, and says to start the export again
  without a cursor, rather than repeat some objects and leave out others.
- **Each page is a Turtle document of its own**: the export's header and
  prefixes again, and its points; a page with no object, while the
  Object_List is still being read, is comments only. A multi-state point's
  enumeration kind belongs to that point, so the pages' points, in order,
  are the whole export's, and concatenated pages make one model. The
  page's first comment says which of the matching objects it holds and
  whether more follow; the header's counts are the page's.
- **Cost.** One request per object, by ReadPropertyMultiple (a device that
  refuses it costs what the CLI section says), plus the device's name on
  every page and the Object_List once (until the cache expires): a small
  device is N + 2 in one page. A point's Turtle runs to 500 to 800 bytes
  (more for a multi-state point with many states), so a page holds about
  30, and a 500-object controller (about 370 kB of Turtle) comes in about
  18 pages, about 519 requests. At the default principal rate (5 a second,
  burst 20) a page takes a few seconds. Every request goes through the
  engine, one at a time under the device's gate, the rates and the
  principal's tokens.
- **A device that stops answering** partway through a page: the page holds
  the objects read before it (its header says the reading stopped early),
  `stopped` holds the error, and `next_cursor` tries that object again. If
  it stops at the page's first object, or at its name, the call is an
  error. One that stops while its Object_List is read element by element
  sets `stopped` too, beside `list.stopped`. A cancel drops the call, as
  for every tool: nothing more is sent.
- **The result.** The first text block is the usual rendering of the
  structured content, which ends by saying the next block is the Turtle;
  the second is the page's Turtle. The structured content has `device`,
  `device_name` (an envelope), `base_iri`, `write_priority`, `listed` (the
  Object_List's length) and `matching` (after `types`, the objects the
  pages go through; both null while the list is being read), `start` (the
  page's first object among them, from 0), `read`, and for the page alone
  `points`, `skipped` (objects whose read the device refused, or that
  couldn't give their name, an analog object's units or a multi-state
  object's state count),
  `by_type` and `unmapped_units` (analog points exported with an unknown
  quantity kind); `turtle_bytes`, `oversized`, `stopped`, `list` (as
  `bacnet_list_objects` gives it), `truncated`, `next_cursor`, `limits`
  and `cost`.
- **Device text.** The Turtle keeps names, descriptions and state texts
  exactly, so a model's `bacnet:object-name` matches the device's, escaped
  so the text stays ASCII (a control or bidirectional character becomes a
  `\u` escape) but not cleaned, defanged or capped as envelopes are. The
  text rendering, and a comment line at the top of the Turtle, say the
  literals are device text: data, never instructions.
- **Limits.** Those of the CLI's [Standard 223
  export](CLI.md#standard-223-export-experimental): a point list, not a
  building model; shape validation warns about the missing sensors and
  equipment; instance 0 loses its identifier string; an unmapped unit gets
  QUDT's unknown quantity kind; U+0000 becomes U+FFFD. A live export isn't
  validated against the shapes.

## Resources

Phase 2 (#2067). Names and titles are built from identifiers only. Content
is JSON (`application/json`): values as `{display, typed}` like the tools,
device text in envelopes, and a `cost` block. A read runs like a tool call,
under its own account and budget, and a cancelled read sends nothing more.

| URI | Content | A subscription means |
|---|---|---|
| `bacnet://devices` | the first page of the discovery table in reach, sorted by instance (up to the device page size, 200 by default), no device text; nothing sent. `total` counts every device in reach and `truncated` says more follow; then `next` holds the next page's URI | an update when a device arrives or is lost, at most once a second |
| `bacnet://devices/after/{instance}` | the next page: the devices whose instance is above `{instance}` (digits only, 0 to 4194303), with the same page size, `total`, `truncated` and its own `next` | the same as `bacnet://devices`: every subscribed page is told when the list changes. A listen stream follows at most 16 device-list pages, the first included and each spelling counted (the acknowledgment leaves out the rest); a legacy session counts them in its 64 URIs |
| `bacnet://device/{instance}` | `bacnet_describe_device`'s summary, cached five minutes (`cached`) | not honoured: a summary has no updates |
| `bacnet://device/{instance}/object/{type}:{n}` (or `{type},{n}`) | the point facts (`bacnet_read`'s `preset: "point"` properties); `source` `read` when read just now, `cov` or `poll` when watched, with `state` (`starting`, `live`, `failed`, `unreachable`, `stopped`), `note` (why it is polled or not live), `seq`, `age_ms` and `updates` (notifications, coalesced, polls, renewals) | a watch on the hub |
| `.../property/{prop}` | one property (`name` or `name[index]`; `[`, `]` and `:` may be percent-encoded) | a property watch (SubscribeCOVProperty) |
| `bacnet://alarms` | each device in reach: its first page of GetEventInformation, as far as the read's budget goes (`unread` past it, `errors` per device) | a poll every 60 s; an update when the events change |
| `bacnet://device/{instance}/alarms` | one device's first page of events | the same, for one device. A session's per-device alarm URIs, on listen streams and legacy subscriptions alike, count against its 64-URI bound (the acknowledgment leaves out the rest), so the poll reads at most that many devices |
| `bacnet://session` | level (`read-only`, `write-session` while a session or plan grant is in force, `write-session (expired)`), `writes` (`not offered`, or the mode, arming and what the client may confirm), `expires` (seconds left in an armed session), `grants` (a plan grant: in force, used, budget, expiry; `used` is null and `count_unavailable` says why when the plan's write count can't be trusted, so its writes are refused), limits, the request meter (`requests_last_10_s`, and of those `agents_requests_last_10_s`, every MCP caller's), the principal's watches and the hub's footprint (points, subscribed, polled, requests, lagged notifications, lifetime); on a session attached to the TUI, `tui.armed`, read-only: whether the TUI's write session is armed, so a write could be confirmed there now | an update when this server arms a session, a write spends a plan grant's budget, or the armed session or the plan grant runs out (announced when it ends, with no request needed) (#2160) |
| `bacnet://ledger` | this principal's ledger entries in effect (`ledger_id`, target, value written, what undoes it, state, time), read from the profile workspace's `ledger.json`; empty without writes | an update when one of this server's writes is recorded as `sending`, and again when it settles (two updates, folded into one when they come close together); when a revert settles; and when it settles an entry left `sending` as unknown. A write the device refused announces its `sending` row and then the row's removal, so the ledger ends as it was. The TUI's own changes to the shared ledger aren't announced (#2160) |

A watched point is read from the hub, which sends nothing. An unwatched one
is read once. Each device's alarms are read at most once a minute, whoever
asks: later reads within the minute come from a cache.

The device list pages by instance, not by position (#2116): `next` names
the last instance listed, so a device discovered between two page reads
moves nothing already seen. It lands on a later page if its instance is
above the cursor, or only in `total` if it is below. MCP's own cursors
page the `*/list` methods only; `resources/read` has none, so the cursor
lives in the URI. Under `--passive` the template isn't listed.

A read refused before anything was sent (a bad URI, a device not discovered
or out of reach, a passive server) is a JSON-RPC -32602 whose `data` is
the tool error body (`invalid-argument`, `unknown-device`, `refused`); one
that failed after sending (the device answered with an error, or not at
all) is a -32603 with the same body.

## Subscriptions

`rmcp` 3.9.0 implements `subscriptions/listen` (2026-07-28): the server
sends the acknowledgment, then `notifications/resources/updated` carrying
only the URI; the client reads the resource. A legacy client (one that
opens with the older `initialize` handshake, such as a 2025-06-18 host)
can't open a listen stream; it subscribes with `resources/subscribe` and
`resources/unsubscribe` instead (#2114), and the `initialize` answer
advertises `resources.subscribe` for every version.

- **Legacy subscriptions.** A `resources/subscribe` is checked as a listen
  URI is and takes a watch on the same hub, under the same caps: listen
  streams, legacy subscriptions and `bacnet_watch` share the 32 points of
  the principal and the 256 of the process. Updates go out as
  `notifications/resources/updated`, the device list and alarm polls
  included. A URI that can't be honoured is a JSON-RPC -32602 with the
  tool error body, as a refused read is: `invalid-argument` (malformed, or
  a device summary, which has no updates), `unknown-device`, or `refused`
  with `device-not-allowed`, `network-not-allowed`, `principal-cap`,
  `process-cap`, `passive` or `shutting-down` (the session is ending);
  nothing is sent. Subscribing twice to one
  URI is one subscription, and a session holds at most 64 URIs (twice the
  point cap), its listen streams' per-device alarm URIs included (#2150). `resources/unsubscribe` drops the subscription and its watch
  at once; the last watch on a point cancels its BACnet subscription. When
  the session ends (the host closes stdin, or a signal) every legacy
  subscription goes with it. The session resource counts them under
  `watches.legacy`.

- **Acknowledgment.** It lists only the URIs honoured. A point URI is
  honoured when it parses, its device is in the discovery table (so call
  `bacnet_discover` first) and in reach, and both caps leave room; an
  over-cap, unknown or malformed URI is visibly left out. The SDK keeps a
  URI asked for twice twice in the acknowledgment; it is watched once.
  The watches are taken when the acknowledgment is built and held for the
  stream; one the stream never claims (the acknowledgment couldn't be
  sent, or the client cancelled first) is released by a sweep within
  about 90 s, and at most 16 such reservations wait at once.
- **Caps.** 32 points per principal (`Caps::watches`; the same point on
  two streams, or a stream and `bacnet_watch`, counts once) and 256 per
  process (`Caps::process_watches`, the hub's cap, which counts points
  still cancelling). Per-device alarm URIs (`bacnet://device/{n}/alarms`)
  count, one a stream each, against the session's 64-URI bound shared with
  legacy subscriptions; past it they're left out of the acknowledgment,
  and the 60-second alarm poll reads only the devices accepted (#2150).
  A session runs at most 4 listen streams at once: a fifth's
  acknowledgment accepts nothing, and its `subscriptions/listen` then ends
  in a JSON-RPC -32602 whose `data` is `refused` with reason `stream-cap`, and it keeps
  no watch. Cancelling a stream makes room.
- **The watch hub** (`bacnet_ops::watch`). One BACnet subscription or poller
  per point however many listeners. SubscribeCOV for an object,
  SubscribeCOVProperty for a property, unconfirmed, with a 300 s lifetime
  renewed 60 s before it ends; a lifetime of zero can't be built. A device
  that refuses COV is polled every 10 s (`source: poll`, with the refusal
  in `note`); a device's polled points are read together, in
  ReadPropertyMultiple requests of up to 16 points and 32 properties (three
  objects' point properties), one point at a time when the answer would be
  too large (#2115). A device that doesn't answer is polled with a backoff
  up to a minute, per device rather than per point (a point whose first
  read went unanswered is polled at once), and read and subscribed again
  after five minutes. Each
  device has one scheduler, which sends one request at a time. The last listener
  leaving cancels the subscription; a listener joining as the last leaves
  gets a fresh entry with its own subscriber process identifier. A point
  whose device isn't in the discovery table yet says so and is looked up
  again after the poll backoff (10 s, doubling up to a minute), then read
  and subscribed once it appears. An object
  watch keeps the point properties: a notification can refresh a row but
  adds one only for a point property, and never past 32 rows, so a device
  can't grow it; a property watch keeps its property, at its element.
  `WatchOptions` also sets what a point reads, the poll floor (1 s at the
  least), the backoffs, whether a point whose device is missing is given
  up, the coalescing window (zero for none) and the sample stream every
  lease can read (`Lease::samples`); the server keeps the defaults.
- **Bursts.** Changes to one point within a second make one notification;
  the folded ones are counted (`updates.coalesced`), and a read always has
  the latest values. Notifications the hub's receiver fell behind on are
  counted as `lagged` in the session.
- **Charging.** The hub's requests (first reads, subscriptions, polls) run
  under its own account and token bucket (the principal rate, 5 a second,
  burst 20), besides the engine's gates and rates, so they never take the
  agent's tokens. A shared point costs the same however many listeners,
  and notifications cost nothing. Renewals take the device's gate but no
  rate slot; cancels skip the hub's bucket. A resource read is charged to
  the principal like a tool call; the alarm polls of a stream are too.
  The bound: at most 256 points (32 per principal) are watched, each
  polled at most every 10 s or renewed once a lifetime, and every hub
  request still waits for its device's gate and the network and global
  rates, so the hub sends at most 5 requests a second beyond renewals and
  cancels however many agents listen.
- **Shutdown.** On a signal (SIGINT or SIGTERM; Ctrl-C on Windows) each
  listen stream completes with its final result, the session closes, and
  every BACnet subscription is cancelled, waiting 10 s plus 60 ms a point,
  at most 30 s (enough for 256 cancels at the global rate); what doesn't
  answer by then lapses with its lifetime, within five minutes. The host closing stdin does the same,
  with nowhere to send the final results.

## The tool-only fallback

For hosts that subscribe neither way (some show resources but never
subscribe), `bacnet_watch` (points: device, object, optional property; up
to 32 a call) puts points on the hub as a subscription would. The first call
returns every value (waiting up to 10 s for new points' first reads); a
call with its `next_cursor` and the same points returns `changed: false`
without values for points unchanged since, and reading them sends nothing.
The cursor holds the hub's sequence number. A point stays watched while
calls ask for it and goes 10 minutes after the last (the cursor's
lifetime), cancelling its subscription. It counts against the same
per-principal cap; a point past it, or a device out of reach, is an error
row while the others are returned.

## BBMD tables

`bacnet_network_map` with `bbmd: true` reads the broadcast distribution
table (BDT) and foreign device table (FDT) of the BBMDs in reach, by
Read-Broadcast-Distribution-Table and Read-Foreign-Device-Table only (Annex
J.2.3 and J.2.7), and returns them under `bbmd` with a verdict.

- **Which BBMDs.** No address range is scanned. The survey starts from the
  BBMD the server registers with as a foreign device (the profile's
  registration) and from up to four `bbmds` the agent names, each the B/IP
  `ip:port` of a device in the discovery table (one the policy allows) or of
  a router heard; any other address is refused (`address-not-known`). From
  there it follows each BDT read to the BBMDs it lists, breadth first, up
  to 16 BBMDs; those past the limit come back as a count with a few
  examples (`unread`). With nothing to start from it is refused
  (`no-bbmd-known`).
- **Peers are device data, so they are filtered.** A BDT peer is on
  network 0 by definition, so following it stays within reach, but its
  address comes from whatever the BBMD holds. An entry that is all zeros
  (a host name the BBMD couldn't resolve), unspecified or port 0, the
  limited broadcast, a directed broadcast (the server's own broadcast
  address, or every host bit of the entry's own mask set: an entry is
  always a BBMD's unicast address), multicast, loopback (unless the server itself is on
  loopback) or the server's own address is never read: it is reported as a
  `not-followed` finding with its reason (`unresolved`, `unspecified`,
  `broadcast`, `directed-broadcast`, `multicast`, `loopback`,
  `own-address`). A named seed must pass the same check, or the call is
  an `invalid-argument`. The TUI's survey
  applies the same filter.
- **Reach and charging.** Every BBMD is on the server's own B/IP network,
  so `[mcp] networks` must include 0 (else `network-not-allowed`); a server
  whose link isn't B/IP refuses (`needs-bip`). Like a router query, it needs
  `broad_discovery` and goes at most once a minute. Each table read is one
  request under the local network's rate, the global rate and the caller's
  tokens. Four BBMDs are read at once, so a silent one costs one 3 s
  timeout without holding up the others, and a read the call's budget
  can't wait for isn't sent. The FDT is read only from a BBMD that gave its
  BDT.
- **The verdict.** `inconsistent` when the BDTs disagree, `incomplete` when
  they don't but some BBMD wasn't read, else `consistent`. Each finding
  names its addresses: `one-way-entry` (`from` lists `to`, which doesn't
  list it back), `missing-own-entry` (a non-empty table without the BBMD itself),
  `no-peers` (an empty table, as a BBMD serving only foreign devices has: a
  note that leaves the verdict alone), `mismatched-masks` (the BBMD and
  every mask the tables give it), `read-not-supported` (a NAK: the BBMD
  refused the read or doesn't support it, as a node that is no BBMD
  answers; not an error), `not-followed`,
  `no-answer`, and `foreign-device-expiring` (under 30 s left, which is
  inside the BBMD's 30 s grace period: the device is past its time-to-live
  without registering again). Addresses are `ip:port`, masks dotted, with
  where peers forward each entry's broadcasts (`forward_to`), and times in
  seconds.
- **Bounds.** Each table lists its first 24 entries with its `count` (the
  check uses them all), at most 64 findings are listed with
  `finding_count`, and `truncated` says when anything was cut. The
  consistency check compares only the tables read, so its work is bounded
  by the 16 reads, not by what the tables list. A survey whose every read
  the call's budget left unsent gives its minute back. The table
  rows, the survey and the consistency matrix are `bacnet_ops::bbmd`'s,
  shared with the TUI's Network screen.

## Writes

Phase 3 (#2068). Off by default: with the `[mcp]` table's `mcp_writes =
"never"` (or none) `tools/list` has no write tool, whatever else the
profile allows. `mcp_writes = "elicit"` lists `bacnet_write` and
`bacnet_revert` (`readOnlyHint: false`, `destructiveHint: true`) and needs
the profile's workspace, which holds the ledger and audit log the TUI uses.
Writes go through `bacnet_ops::safety`: the TUI's planner, tiers, ledger
(entries marked `front_end: mcp` with the principal) and audit log.

- **`bacnet_write`**: `device`, `writes` (1 to 20: `object`, `property`
  with an optional `[index]` (`present-value` when left out), `value`,
  `priority`), `batch`
  (WritePropertyMultiple; several writes need it) and `reason`. Targets are
  numeric (`ao:1` or `ao,1`, never a name); a value is a number, a boolean,
  `null` (relinquish), a typed `{"value": n}` or the
  [CLI value grammar](CLI.md#writing-properties), checked
  before anything is sent. A Priority_Array element is never written
  (`read-only-property`: command or relinquish the slot through
  `present-value` at that priority), and a write to Relinquish_Default is
  T3 over MCP, so only a plan grant can approve it. The result names each write's `outcome` (the
  TUI's outcome JSON), `applied` (true, false, or null when unknown) and
  `ledger_id`, with a `revert` hint; when a write isn't acknowledged it is
  an error result (`error`: `timeout` with `applied: "unknown"`, `bacnet`,
  `failed` or `refused`) holding the same rows.
- **Approval.** The first call reads the targets (the TUI's pre-read) and
  previews the change. A blocked change (priority 6, which MCP never
  allows) is refused (`priority-6`). A plan grant in force approves the
  writes it covers at once, under the workspace's lock; one it doesn't
  cover is refused (`outside-plan`). Otherwise the client may approve T1
  only (priority 9 to 16, or a property that isn't commandable): T2 and T3
  are refused (`needs-verified-approver`). A T1 write returns
  `input_required` (multi round-trip requests, 2026-07-28) with a form
  elicitation `confirm` (the canonical targets, the value now and after,
  the Present_Value change and the planner's notes; device text cleaned and
  labelled) and a sealed `requestState`. An older-revision session that
  declared form elicitation (or the capability without a mode, as
  2025-11-25 and earlier clients do) is asked by the server instead, with
  an `elicitation/create` request carrying the same form (#2158): the
  call waits for the answer, holding no lock, until 5 s before the sealed
  state would expire (115 s), then goes on as the retry would. Accepting
  sends the write (`confirmed_via: "server-elicitation"`); declining or
  cancelling is recorded as `declined`, nothing sent. No answer in time,
  an error answer, the call cancelled or the server shutting down is
  refused (`confirmation-expired`, the audit saying which), nothing sent,
  and the request is cancelled at the client. One such request waits at a
  time: meanwhile another write from that client that no plan grant
  covers is refused (`confirmation-pending`) before anything is read,
  unasked and with nothing sent. A client that
  declared neither, or URL elicitation only, is refused
  (`elicitation-unsupported`).
- **Arming.** A write needs an armed session or a plan grant in force, as
  in the TUI. With `mcp_arm = "elicit"` the confirmation of a write while
  unarmed arms a 15-minute session too (audited `arm`); with `never` (the
  default) only a plan grant arms MCP writes (`not-armed`). Never
  `dangerous`.
- **The retry** carries the form's answer and the state. The state is
  XChaCha20-Poly1305 under a key drawn per process, with the principal and
  the tool as associated data, and holds a digest of the arguments, a
  digest of the preview, a nonce and a 120 s expiry (the process's own
  clock). A forged or changed state, or another caller's
  (`confirmation-invalid`), an expired one (`confirmation-expired`), one
  used before (`confirmation-replayed`; a state opens once even when the
  retry is then refused) and different arguments (`confirmation-mismatch`)
  are refused and audited, with nothing sent. A declined or cancelled form
  is audited `declined`. The confirmed write runs on a task of its own: it
  takes the workspace's lock (refused `workspace-locked`, naming the
  holder, when the TUI has it), reads the targets again, refuses a preview
  that changed (`stale`, audited as a stale decline), records each write as
  `sending` in the ledger and the audit log, sends, and settles; a timeout
  leaves the entry `unknown`. The lock is released after. A planned write
  and a revert run on such a task too. Once a write is handed to its task
  the call never answers that nothing was sent: a cancel only stops the
  waiting, and a call past its budget answers `error: "pending"` with the
  `ledger_ids` recorded so far, `bacnet://ledger` to follow and no retry
  hint: `applied: "unknown"` once an entry is recorded, `sent: false`
  before. The tasks are tracked: a shutdown waits for them, at most one
  call's budget plus its 15 s grace (45 s by default), so their outcomes
  reach the ledger and the audit log. Whoever takes the workspace's lock
  settles every entry left `sending` as `unknown` (audited), whatever run
  wrote it: a writer in flight, in any process, holds that lock until it
  has settled (the TUI for its whole session, MCP for each write), so such
  an entry has no writer left. A task that panics is settled that way at
  once.
- **`bacnet_revert`**: `entries` (ledger ids) or `all`, this principal's
  MCP entries only (`not-yours` for another's), on devices the policy
  reaches; no prompt and no armed session, as in the TUI. Latest write
  first, through the desk; the result names each entry's outcome and how
  many are `left`.
- **Plan grants** (`[mcp.plan]`: `devices`, `properties` (`present-value`
  by default), `priority_min` and `priority_max` (8 and 16 by default),
  `max_writes` (1 to 10,000), `hours` (1 to 8), `reason`) run from
  `bacnet mcp authorize` (never from a later time than now, so a
  hand-edited time can't lengthen one; the server refuses a plan running
  more than 8 hours from start), which spells out what the plan lets
  through without a prompt and warns when its range includes priorities
  1, 2 or 5. The budget is counted in the profile workspace beside the
  ledger (`plan-budget.json`, #2159), for that authorization, so a
  restart doesn't reset it and authorizing again starts it at zero. Each
  write the plan covers is counted, under the workspace's lock, before it
  is sent; a count that is missing, unreadable, corrupt, left by another
  authorization, or can't be saved refuses the plan's writes
  (`plan-budget-unavailable`, nothing sent) until `bacnet mcp authorize`
  runs again; the server logs a warning at its start when it finds no
  count to trust, and `bacnet://session` shows why. While a server runs,
  the count it has seen is a floor, so a count file put back from a
  backup can't hand writes out again. A plan that ends between a call's
  check and its write is refused (`plan-spent`). `bacnet mcp
  serve` warns when writes are offered with `mcp_arm = "never"` and no
  plan, since every write is then refused (`not-armed`).
  Offering writes, letting the client arm, or adding or changing a plan
  clears the authorization (dropping a plan doesn't).
- **Audit.** Every MCP record has `front_end: "mcp"`, `principal`,
  `client` (`claimed_name`, a claim), `tool`, `args_digest` and
  `request_id`, and a write `confirmed_via` (`elicitation`,
  `server-elicitation` or `plan`) and
  `approved_by` (`client` or `plan`, with the plan's reason and count).
  Refusals and declines are recorded without the lock: the audit log is
  append-only.
- Not offered: DeviceCommunicationControl, ReinitializeDevice, and every
  service the design excludes.

## Passive mode and resources

`--passive` lists only `bacnet://session` and `bacnet://ledger` and no
templates; reading any other resource is refused (`refused: passive`), and
a listen stream or a legacy `resources/subscribe` honours only those two.
Nothing that needs traffic is offered.

## Results

- Every result carries `cost` (`requests`, `elapsed_ms`, `throttled_ms`,
  `timeouts`) and, for success, `limits`; paged results carry `truncated`
  and `next_cursor`.
- Errors are `isError: true` results whose `error` says what happened:
  `invalid-argument`, `refused` (with `reason`, and `retry_after_s` when
  rate-limited) and `unknown-device` sent nothing; `bacnet` (the device's
  Error, Reject or Abort, kebab-case names with their numbers), `timeout`
  and `failed` did. An unknown tool or arguments outside the schema are
  JSON-RPC errors (-32602), and so is a tool this configuration doesn't
  list (a passive server's read tools). A test checks every tool's
  results, success and error, against its output schema. A cancelled call
  (`notifications/cancelled`) drops its future, so it sends nothing more.
- Values are `{display, typed}`: the shared formatter's text, and its
  typed form, `bacnet_ops::format::Datum`, the shape the CLI's JSON uses
  (numbers as numbers, `{value, name}` for enumerations, `{bits, names}` for
  bit strings, `{type, type_number, instance}` for objects, `{element,
  text}` for constructed elements, with `fields` for an Event_Parameters
  value: its algorithm and each member by name, each a typed value). Raw
  hex only for octets that don't decode.
- Device text is an envelope, leaf by leaf: names, descriptions, vendor
  strings, a character-string value in `typed`, and the `text` of a
  constructed element whose kind can hold a string (a name and value, a
  schedule's values, an access result, an unnamed kind), but not of one
  that can't (a time stamp, a date and time, a reference, a date range:
  the kinds in `bacnet_ops::format::PLAIN_ELEMENTS`). An element's
  `fields` are values in turn, so a character string among them (a
  change-of-characterstring alarm value) is enveloped too. The display text is
  enveloped whole only when the value holds device text. Each envelope is
  `{untrusted, source, truncated}`: C0 and C1 controls and the line and
  paragraph separators become spaces; format characters (category Cf:
  bidirectional, zero-width, tag characters and the rest), variation
  selectors and Hangul fillers are dropped; `http://` becomes
  `hxxp://` (other schemes `scheme[:]//`), and the text is capped (256
  characters by default). The 223 export's Turtle keeps device text
  exactly, escaped, outside any envelope.
- Cursors are `hex(body).hex(HMAC-SHA256)` under a key drawn per process,
  binding the tool, a digest of the principal (the OS user's name doesn't
  show), a digest of the other arguments, an expiry (10 minutes) and the
  position.

## Profile keys

Every key of a profile's `[mcp]` table is optional; an unknown key fails the
profile's load. The defaults are `DEFAULT_RATES` and `DEFAULT_CAPS` in
`crates/bacnet-cli/src/tui/profile/mcp.rs`, which a test holds equal to
`bacnet_mcp::policy::{Rates, Caps}`. The limits are `Policy::validate`'s: a
value outside them stops the server from starting. Rates are requests a
second.

| Key | Meaning | Default | Limits |
|---|---|---|---|
| `networks` | Networks the tools may reach; 0 is the local one | all | not 65535 |
| `devices` | Device instances it may reach, each `"low-high"` or `"N"` | all | 0 to 4194303, low no higher than high |
| `slow_networks` | Networks held to `slow_rate`, such as a slow MS/TP trunk behind a router | none | not also in `fast_networks` |
| `fast_networks` | Networks held to `fast_rate` | none | not also in `slow_networks` |
| `broad_discovery` | A global, unbounded or wide Who-Is (and `bacnet_network_map`'s `send`), one a minute | `false` | |
| `max_rate` | Across all networks | 20 | 1 to 1000 |
| `network_rate` | To any one network | 20 | 1 to 1000 |
| `slow_rate` | To a network in `slow_networks` | 2 | 1 to 1000 |
| `fast_rate` | To a network in `fast_networks` | 5 | 1 to 1000 |
| `principal_rate` | For the agent | 5 | 1 to 1000 |
| `principal_burst` | Requests the agent may send at once | 20 | 1 to 1000 |
| `max_values` | Property values one read returns | 100 | 1 to 1000 |
| `page_devices` | Devices on a discovery page | 200 | 1 to 2000 |
| `page_objects` | Objects on an object-list page | 200 | 1 to 2000 |
| `max_text` | Characters kept of a device-supplied string | 256 | 16 to 4096 |
| `call_budget_s` | Seconds a call may wait on the limits and the network | 30 | 1 to 600 |
| `mcp_writes` | `"elicit"` offers `bacnet_write` and `bacnet_revert` ([Writes](#writes)), on stdio and through the TUI ([Attached to the TUI](#attached-to-the-tui)) | `"never"` | `"never"` or `"elicit"`; `"elicit"` needs the profile's workspace |
| `mcp_arm` | `"elicit"` lets the client's confirmation arm a write session | `"never"` | `"elicit"` needs `mcp_writes = "elicit"` |
| `tui_listener` | Whether `bacnet tui` listens for agents on its local socket while the profile is open ([Attached to the TUI](#attached-to-the-tui)) | `true` once MCP is authorized | `true` or `false`; never listens without the authorization |
| `[mcp.plan]` | Writes approved in advance ([Writes](#writes)): `devices` and `max_writes` (1 to 10,000) and `hours` (1 to 8) are required; `properties` defaults to `["present-value"]`, `priority_min` to 8 and `priority_max` to 16 (both 1 to 16); `reason` is optional | none | needs `mcp_writes = "elicit"` |

`bacnet mcp authorize` writes `authorized_at_unix`, `[mcp.authorized]` and
`authorized_transport`; don't edit them by hand ([Operation](#operation)).
A change that raises a rate or a cap, adds a network or device, enables
broad discovery, writes or arming, changes a plan in any way, takes a
network out of `slow_networks` or `fast_networks`, or turns `tui_listener`
back on clears that consent.

## Limits

Every request goes through `RequestLimits`: the caller's token first (5 a
second, burst 20), then the device's gate (one outstanding request), then
its network's rate (20, or 2 for `slow_networks` and 5 for
`fast_networks`; a router doesn't say what link it reaches), then the
global rate (20). A request books its slots and token as it starts
waiting; one cancelled before it goes, whichever of those it is waiting
for, gives them back, so later requests don't wait behind it. Each tool
call runs on its own task under an `Account` that counts its requests and
waits. A call that pages stops at its budget (30 s) and returns a cursor;
one still waiting `budget + 15 s` later returns `refused: rate-limited` if
it sent nothing, else `timeout`.

## Operation

- Stdout carries only JSON-RPC. `mcp.rs` denies `print_stdout` and
  `print_stderr` though the CLI allows them, tracing goes to stderr, and
  `crates/bacnet-cli/tests/mcp_stdio.rs` checks every stdout line.
- Live traffic needs a profile (maintainer decision, 2026-10-25). Without
  one, `serve` runs only with `--passive`; with one, transport flags are
  refused, since they would change the transport that was authorized.
- A profile serves MCP only once `bacnet mcp authorize` (a person typing
  the profile's name in a terminal) records, in the `[mcp]` table,
  `authorized_at_unix`, the table it covered as `[mcp.authorized]`, and the
  profile's transport settings as `authorized_transport`. A time without
  both authorizes nothing. Loading a profile whose table now does more
  (more networks or devices, broad discovery, a higher rate or cap, a
  network no longer slow or fast) or whose transport changed clears it.
- Reads are logged on stderr as summaries, always: one `read call` line
  per call (tool, device, requests, outcome) and `read summary` counts per
  device at shutdown. The `bacnet_mcp::reads` target stays on at info
  whatever `-v` or `RUST_LOG` say; they set the rest. They are not written
  to the TUI's `audit.jsonl`.
- `--passive` builds no client; only `bacnet_discover` (which refuses to
  send) and `bacnet_decode` are listed, and only the session and ledger
  resources.
- SIGINT or SIGTERM (Ctrl-C on Windows) shuts the server down gracefully:
  listen streams complete and COV subscriptions are cancelled.
- A request whose handler panics (a bug) is answered with an internal
  error, and `serve` then exits with an error naming the panic, rather
  than serve on with whatever state the panic left (#2203).

## Attached to the TUI

Phase 4 (#2185; the Windows named pipe, #2201). While
`bacnet tui` has a profile open, agents can attach to it instead of
starting their own client: `bacnet mcp connect --profile NAME` passes its
stdin and stdout to a local socket (on Windows, a named pipe) in the TUI's
process, byte for byte, and the agent host
launches it as it launches `bacnet mcp serve`. Without `--profile`, it
attaches to the only profile a TUI is listening on (one whose listener
holds its lock, below), and asks for a name when there are several. With no TUI listening it exits non-zero, saying so on
stderr; when the TUI ends the session it exits cleanly.

- **When it listens.** Opening a profile that is authorized for MCP
  (`bacnet mcp authorize`) starts the listener, unless its `[mcp]` table
  sets `tui_listener = false`; a profile authorized before this release
  starts one too after the upgrade, for your own user, and the log pane
  says so. Its agents write only if the table already set `mcp_writes =
  "elicit"`, and then only what you confirm in the TUI with its write
  session armed. An unauthorized profile, a passive session, an
  MS/TP monitor, or a command line whose transport flags change the
  authorized transport gets none, and the log pane says why whenever the
  profile asked for one. The status bar shows `MCP n`, the agents
  attached, while it listens, and `MCP off` when a listener the profile
  asked for couldn't start.
- **One client.** Each connection gets its own MCP session, as a stdio
  server does, under the profile's `[mcp]` reach and caps, as the TUI's
  OS user. Its reads go through the TUI's engine: the same client,
  transport and UDP port, the same device gates, and the watch list's
  rate meter counts them. The agents are one principal, so their sessions
  share one `principal_rate` budget, one watch hub (256 points between
  them, as for one stdio server), one per-principal point cap, and the
  once-a-minute limits on broad Who-Is, router queries and BBMD surveys;
  the hub's subscriptions use
  subscriber process identifiers of their own, so an agent leaving a
  point the watch list also watches never cancels the TUI's subscription.
- **Limits while an agent is attached.** With no agent attached the TUI
  keeps to its own `--max-rate`. From the first agent's connection until
  the last one leaves, the table's rates join it: the global cap is the
  lower of the TUI's and `max_rate`, and `network_rate`, `slow_networks`
  and `fast_networks` pace every request to those networks, the TUI's own
  included. The log pane notes each switch.
- **Writes, approved in the TUI.** Read-only unless the table sets
  `mcp_writes = "elicit"`; then the socket (or pipe) lists `bacnet_write`
  and `bacnet_revert`, and the person at the TUI approves every write, at
  every tier, as they would their own. The client's confirmation,
  `mcp_arm` and a `[mcp.plan]` play no part here (a plan grant still
  works on stdio).
  - A write's first call reads the targets and previews the change at its
    tier over MCP (a Relinquish_Default write is T3; priority 6 is
    refused). The call then waits, outside its `call_budget_s`, for the
    TUI's answer.
  - The request opens the TUI's preview dialog under an "agent request"
    banner naming the agent (its connection number, and the name its
    client claims). The dialog is the TUI's own: a key within 300 ms of it
    opening is ignored, T1 takes Enter, T2 `y` and T3 the device instance
    typed out, and after 120 s it is declined.
  - The TUI's write session must be armed (`W`), and its profile mustn't
    set `writes = "never"`; otherwise the request is refused at once
    (`not-armed`, `write-not-granted`), unshown.
  - Requests queue one at a time, oldest first, with no "approve all". The
    banner and the status bar (`MCP n, m to confirm`) show the queue. It
    holds 4, the one on screen included; one more is refused
    (`approval-queue-full`). A request waits behind another dialog the
    person has open.
  - Disarming the TUI (`W`, or the write session running out) refuses
    every request it holds at once, the one on screen included, with
    `not-armed` and nothing sent, rather than leaving them to be refused
    when confirmed.
  - **Host timeouts.** Agent hosts often end a tool call after about 60 s,
    while the dialog waits up to 120 s, and a request queued behind
    others waits longer. A request answered after the host gave up is
    still sent if confirmed, but the agent never hears the result; it can
    read `bacnet://ledger`. Raise the host's tool timeout where it has
    one, and answer requests promptly; the agent pane shows each one's
    age.
  - Confirmed, the write runs on the TUI's own write session: the targets
    are read again (a preview that changed is refused `stale`), the
    ledger records it as the agent's (`front_end: mcp` with the
    principal, so `bacnet_revert` can undo it, and the TUI's `R` too),
    then it is sent and audited with `confirmed_via: "tui"` and
    `approved_by` the TUI's OS user. The call returns the outcome as on
    stdio, and the TUI flashes it.
  - Declined, timed out, the TUI quitting or its session ending, the call
    returns `declined`, with nothing sent, and the agent's session audits
    it (`event: "declined"`, `asked_via: "tui"`). Device text and values in
    the dialog are shown cleaned of control and invisible characters and
    cut to 80 characters, and the client's claimed name loses any quotes. An agent that cancels
    its call (`withdrawn`) or disconnects has its request taken off the
    queue, or its dialog closed. However the TUI or the agent ends, no
    call waits longer than the queue's full turn (490 s).
  - `bacnet_revert` is confirmed in the same dialog (the banner says it
    reverts): it shows the writes that put the pre-images back, each at
    the tier of the write it undoes (T1 at least; undoing a
    Relinquish_Default write is T3, as the write was), and the TUI's write
    session must be armed, checked again just before anything is sent
    (`tui-refused` otherwise). The desk sends exactly what was confirmed:
    it rebuilds the revert from the ledger as it is then, and refuses it
    as `stale` when that differs (an entry id another write has taken
    since, or a later write to the slot merged into the entry), with the
    device checks run again. Its scope is every MCP entry of the TUI's
    OS user, the principal: every agent's, through the TUI or on stdio,
    not only this agent's. The dialog confirms one device, so the
    entries named (or all of them) must be on one device
    (`several-devices` otherwise).
  - A key pressed within 300 ms of the dialog's first frame is dropped,
    digits typed toward a T3 confirmation included, measured on the wall
    clock from that frame. A request that arrives while a filter is being
    typed waits until the filter is done, so the dialog never takes its
    keys.
  - `no-approver`: the TUI's session ended (or wasn't connected) before
    the change went, so nothing was sent. A dropped job whose write had
    reached the ledger answers `pending` with `applied: "unknown"`
    instead.
- **The agent pane.** Screen 8 (`8`, listed on the top line while the
  listener runs) shows the attached agents' tool calls, newest first:
  the time (UTC), the agent (its connection number and the name its
  client claims), the tool, a short summary of the target, the outcome
  (`ok`, `refused:` and the reason, `declined` or `withdrawn`, `stale`,
  or `error:` and its kind) and the cost: the requests charged to the
  call, the re-read before a write and the writes sent included, and how
  long it took. It keeps the latest 500 calls; older ones are dropped,
  and the heading counts them. Above the calls it lists the write
  requests waiting for you, each with its age, the one on screen with
  the seconds left before it is declined, and a warning once a request
  is older than a host's usual 60 s. The heading gives the agents' share
  of the request meter, and the watch list's meter adds the same share
  (`agents n/s`): a request counts as an agent's when it is charged to an
  agent's call or to the agents' watch hub, so the pane and the meter
  agree. The sessions record each call on the listener's threads; on each
  tick of its loop the TUI copies the list if it changed, skipping the
  tick rather than waiting when a session holds it. Scrolled down, the
  pane keeps the same calls in view as new ones arrive.
- **The socket (Unix).** `mcp.sock` in the profile's workspace. The workspace
  must be a directory of yours, not a symbolic link; one with a looser
  mode than 0700 is narrowed to 0700 when the listener starts, and the
  log pane says so (new workspaces are made 0700). The socket is made
  0600, and every connection's peer credentials (`SO_PEERCRED`, or
  `getpeereid` on macOS and the BSDs) must name the TUI's own user before
  any MCP message is read. A refused connection is closed and audited. A
  TUI running as the overflow uid (65534), which peers in other user
  namespaces report, doesn't listen. A socket path longer than the OS
  takes (103 bytes on macOS and the BSDs, 107 on Linux) is refused before
  binding: use a shorter profile name or point `BACNET_CONFIG_DIR`
  somewhere shorter. At most 8 agents attach at once, and a connection
  that sends no MCP message within 30 seconds is closed (once open, a
  session may sit idle). `bacnet mcp connect` probes the lock with a
  shared lock, and a starting listener retries for a moment, so a probe
  never keeps a TUI from listening.
- **The named pipe (Windows).** `\\.\pipe\rusty-bacnet-mcp-` and a hash
  of a salt, a nonce of the TUI's run and the workspace's canonical
  path, so each profile has its own and `connect` finds it with no
  setting (#2219, #2237). Pipe names are one namespace for every user of
  the machine, so the salt keeps another user from working the name out
  in advance from the path. Neither hides the name while the TUI
  listens, since any local user can list the pipes then; but each time
  the TUI starts listening it picks a new nonce, so a name seen during
  one run is useless for the next: a pipe someone makes under it while
  the TUI is closed doesn't stop the TUI listening again, under another
  name. While the TUI listens, its first instance holds the name and
  nobody else may add one (see below). The nonce is 32 random bytes in
  `mcp-pipe.nonce` in the workspace. The TUI writes it after taking the
  workspace's lock and claiming the pipe's name: to a new file owned by
  its user and open to nobody else from creation, synced, then renamed
  over the last run's, so a reader finds one whole file or the other.
  A rename that fails for a moment (a reader holding the old file, say)
  is tried twice more; one that keeps failing leaves the TUI not
  listening, with an error that says to try again. It removes the file
  first thing as it stops; one left by a TUI that crashed names a pipe
  nobody holds, and `connect`, which reads the nonce on every connect,
  says that no TUI is listening, as it does with no nonce file. Should
  someone who saw a crashed TUI's name make a pipe under it, `connect`
  refuses it with the message naming its owner or user, and passes it
  nothing; that is safe. The salt stays, though the nonce alone would
  hide the name: it costs one read, and keeps the name unknown to others
  should a nonce ever be reused. The salt is 32 random bytes in
  `mcp-pipe.salt` in the workspace, made by
  the first TUI to listen there and owned by its user. Two TUIs starting
  at once agree on one salt: each writes its own under a temporary name
  and links it into place, and the one that finds a salt there already
  takes it. The TUI refuses a workspace that is a junction or symbolic
  link, that isn't owned by its user, SYSTEM or the Administrators
  group, or that lets anyone else list or change it (the error shows an
  `icacls` command to fix it). The TUI and `connect` both refuse a salt
  file that isn't a plain file, isn't owned by the user, grants anyone
  but the user, SYSTEM and the Administrators group access, or doesn't
  hold exactly 32 bytes, and `connect` refuses a nonce file by the
  same rules. The salt is never made anew while one is
  there, since a running TUI's pipe name depends on it: remove a refused
  one with no TUI open on the profile. With no salt file, `connect` says
  that no TUI has listened on the profile.
  The pipe's security descriptor is owned by the TUI's user and grants
  access to that user's SID alone, with no entry for Everyone,
  Authenticated Users, anonymous logons, SYSTEM or Administrators (the
  agents are your own processes, and SYSTEM and administrators can take
  any object over through their privileges anyway, so an entry for them
  would only widen access). Its medium mandatory label keeps a process
  of lower integrity, such as a sandboxed one of the same user, from
  opening it to read or write. There is no entry denying network logons:
  remote clients are refused anyway, and a local session that holds the
  network logon SID (an SSH session, say) would be shut out of its own
  TUI. Every instance refuses remote clients. The first instance claims
  the name, so a pipe of that name made by anyone else (another user's
  process, say) is refused rather than joined, and
  the next instance is made before a connected client is handed to its
  session, so the name never goes free while the TUI listens. Before any
  MCP message is read, the client's process (`GetNamedPipeClientProcessId`)
  must run as the TUI's user, by its token's user SID; one that doesn't,
  or whose user can't be found, is closed and audited. `bacnet mcp
  connect` checks the listener's process the same way
  (`GetNamedPipeServerProcessId`), and that the pipe itself is owned by
  the same user, before it passes any byte on, so an impostor pipe gets
  nothing, and it lets the listener identify it but
  never act as it. A pipe has no half-close, so on Windows `connect`
  ends its connection as soon as the host closes stdin. The limits on
  agents and on the handshake are those of the socket.
- **The lock.** A listener holds an exclusive lock on `mcp.lock` in the
  workspace for its whole life, taken before it touches the socket path
  or makes the pipe.
  A second TUI on the same profile finds it held and doesn't listen; the
  first keeps serving. On Unix, holding the lock, any socket at the path
  is one a crashed TUI left, and it is removed; a symbolic link or any other file
  there is neither followed nor removed.
- **Audit.** Each connection's open (`mcp-connect`), close
  (`mcp-disconnect`) and each refused peer (`mcp-refused`, with the peer's
  uid, or on Windows its SID, its pid and why) go to the workspace's
  `audit.jsonl`, with `via` set to `local-socket` or `named-pipe`, and
  `front_end: "mcp"` and the principal. At most 10 refusals a minute are
  recorded; the number left out goes with the next refusal record
  (`refusals_not_recorded_before`) and in one record at shutdown.
- **A bug in a session.** The agents' sessions, and every task their
  requests start, run on threads of the listener's own (named
  `bacnet-mcp-agent-N`). A panic there is logged and leaves the screen
  alone; that agent's session ends, with its `mcp-disconnect` record
  giving the panic, while the TUI and the other agents carry on. A
  request whose handler panics answers with an internal error first; on
  stdio, `bacnet mcp serve` exits the same way (see Operation).
- **Ending.** Closing the profile or the TUI stops the listener: each
  agent's listen streams complete, its connection closes, the shared
  watch hub cancels its COV subscriptions, the sessions' threads stop,
  the socket file (or every pipe instance) is removed and the lock
  released. On Windows the run's nonce is withdrawn first, so a
  `connect` during the shutdown already hears that no TUI is listening.

## Tests

- `crates/bacnet-mcp/src/tests/`: the stdio protocol over a pipe against
  an in-process lab (handshake, tool list, survey, errors, cursors, the
  2026-07-28 request form), no write tool unless the profile offers writes, passive
  mode, 500 reads within the global cap, one request outstanding per slow
  device, a slow network held to 2 a second, the injection fixture, the
  policy's reach, broad discovery (wide ranges, ranges outside the policy,
  parallel calls), network numbers, network-paced Who-Is, cancellation, a
  long indexed Object_List across calls, and output schemas (success and
  error results of all twelve read tools, nested values included; the
  write tools' in `writes.rs` and `plan.rs`).
- `crates/bacnet-mcp/src/tests/export.rs` (#2436): the 223 export of the
  lab (an analog input with its unit, a binary output, a multi-state value,
  an unmapped unit) in one page passing the export's structural check,
  with its counts and N + 2 requests; a 136-point device in pages whose
  points concatenate to the whole export's, each object once and each
  page reading only its own; the base IRI, types and write priority; bad
  options and other calls' cursors refused with nothing sent; a list
  grown or shrunk between pages refused after the list read; a spent
  budget ending a page that the cursor resumes; a device silent mid-page;
  the policy's object page; an Object_List read element by element across
  calls; a cancel sending nothing more (through the handler, and over the
  pipe, where it goes unanswered). `tools/export_tests.rs`: a 500-object
  controller in 18 pages under the text cap, an oversized point mid-page
  going out alone, the page cap, and the cursor's position.
  `tests/discovery.rs`: a slow element-by-element list read, calls 200 s
  apart, completing and then served from the cache.
- `crates/bacnet-mcp/src/tests/bbmd.rs`: real BBMDs on loopback. A survey
  from the registration's BBMD reports a missing peer entry, a refused
  read, a silent address (one timeout) and a foreign device near expiry,
  and the wire shows only table reads; a known device's address seeds a
  survey that names mismatched masks; unknown addresses, a device outside
  the policy, a network outside it, no broad discovery, a second survey
  within the minute and passive mode are refused with nothing sent.
- `crates/bacnet-mcp/src/tests/site.rs` and `trends.rs` (part B): finding
  objects by name, type and object, cached names, pages across devices,
  reach and skipped devices; the network map, its broad-request rule and
  its reach; alarms paged with summaries; the decode injection fixture in
  passive mode; trend paging across ReadRange answers cut short and across
  calls, the summary against a brute-force computation (acceptance 7), time
  windows, states, a spent budget resuming without loss, and cancellation.
  `logs.rs` (#2103): an Event Log and an Audit Log paged across ReadRange
  answers cut short and across calls, every record once and in order; the
  per-type summaries against brute-force counts; the message and comments
  of a hostile record enveloped; other objects refused; a time window.
  `limits.rs`: a device silent mid-list or while naming, a list cut by the
  budget, alarms stopped by the budget, the router wait ending with the
  budget, a cancelled router query, and a backwards trend window.
- `crates/bacnet-mcp/src/tests/listen.rs` and `resources.rs` (phase 2):
  two streams on one point make one subscription in the lab device's own
  table (Active_COV_Subscriptions) and the last one cancelled removes it; a
  watched point read without sending; the acknowledgment leaving out
  over-cap, unknown, out-of-reach and malformed URIs; a point the device
  won't subscribe polled with `source: poll`; a storm of 30 changes making
  at most 3 notifications with the latest value read after; a graceful
  shutdown completing two streams at once and cancelling every
  subscription; a silent device reported unreachable; passive mode; the
  device list announced once on arrivals; the resource lists, contents,
  envelope, caches and a cancelled read; `bacnet_watch` across cursors,
  its idle expiry, cap and reach; output schemas include `bacnet_watch`.
- `crates/bacnet-mcp/src/tests/legacy.rs` (#2114): a 2025-06-18 session
  offered `resources.subscribe`, subscribing (twice, one subscription),
  told of a COV change, and unsubscribing, which cancels the subscription
  in the lab device's table; closing the session, and dropping its future,
  releasing every watch; the caps shared with listen streams; refusals as
  -32602 errors with nothing sent; passive mode; the device list announced.
- `crates/bacnet-mcp/src/tests/paging.rs` (#2116): `next` followed from
  the first page lists every lab device once, in instance order, at page
  sizes 3, 5, 10 and 200, with no `next` on the last page even when full;
  devices found between page reads repeat and skip nothing; bad cursors
  (non-digits, over 4194303, a leading `+`) refused with nothing sent; no
  template and no page under `--passive`; every followed page announced
  when a device arrives (listen streams here, legacy subscriptions in
  `legacy.rs`); a stream asking for more than 16 pages honoured for 16,
  with one update per page on a change; legacy pages counted against the
  session's URI bound.
- `crates/bacnet-ops/src/watch/tests.rs`: the hub against the device's
  subscription table: sharing and cancel, a refusal polled every 10 s, a
  coalesced storm, renewal before expiry, a silent device and recovery, a
  property watch, the cap with entries still cancelling, a listener
  joining as the last leaves, shutdown, and no lifetime of zero.
  `scheduler_tests.rs` (#2115): a device's polls batched, a batch too
  large asked a row at a time, RPM refused and remembered, a failed batch
  read property by property, the silent-device backoff, subscription
  retries and their cancels, a missing device given up, what a point
  reads, the sample stream, a zero coalescing window, and the routed
  notification match.
  `lifecycle_tests.rs`: the first notification after subscribing kept,
  shutdown within its wait with a poll in flight, an arrival as a
  scheduler ends, a one-row batch too large to answer, the cancel of a
  point given up, and a renewal behind a busy gate.
- `crates/bacnet-ops`: network rates, accounts and buckets, a trend page
  charged per request, Who-Is through the engine, and the decoder's tests
  (moved from the CLI).
- `crates/bacnet-cli`: the `[mcp]` table, authorization and its
  revocation, the write policy and plan grant from the table, and the stdio
  hygiene test against the real binary. The TUI lists an MCP entry from the
  shared ledger marked `[mcp: user]` (`tui/tests/writes_app.rs`).
- `crates/bacnet-mcp/src/tests/writes.rs` and `plan.rs` (#2068): the write
  tools absent with `never` and listed with `elicit`; a T1 write previewed,
  confirmed, sent, ledgered, audited and reverted; T2, T3 and priority 6
  refused; tampered, expired, replayed, mismatched and declined
  confirmations sending nothing; a stale preview; a timeout leaving an
  `unknown` entry; no arming, an unsupported client and the TUI's lock
  refused; 50 plan-grant writes without a prompt, each audited as approved
  by the plan, and writes outside it refused. `writes/seal.rs`: a state
  bound to its principal and tool, opened once, within its lifetime.
- `crates/bacnet-mcp/src/tests/attach.rs` (#2185; Unix, and on Windows
  the reads, the read-only list and the shutdown): an agent
  through the bridge reads the lab on the shared engine, which its rate
  meter shows, with no write tool though the policy offers writes; a peer
  of another uid (named by a stand-in for the kernel's credentials, since a
  test can't connect as another user) closed before any MCP traffic and
  audited with its uid and pid; a loose workspace narrowed to 0700, a
  symbolic link or a file refused; a second listener refused by the lock
  while the first keeps serving, a stale socket cleared, and a symbolic
  link or file at the socket's path left alone; `bacnet_revert` refused
  like `bacnet_write`; shutdown completing a listen stream, cancelling its
  COV subscription, ending the bridge and removing the socket.
  `tests/attach_more.rs`: the `[mcp]` limits only while an agent is
  attached; four flooding agents held to one budget while the TUI's reads
  finish; the TUI keeping its COV notifications when an agent leaves the
  same point; the bridge refusing a listener of another user with nothing
  written and unreadable credentials refused and audited (both Unix); an
  idle connection closed after 30 s. `tests/attach_shared.rs`: a second session's broad
  Who-Is within the minute refused and the watch cap counting both
  sessions' points; shutdown not waiting out a silent handshake; an agent
  keeping a shared point's COV updates when the other leaves; an open
  session idle past 30 s kept; a ninth agent refused (`too-many-agents`)
  until a seat is free. `attach_more.rs`, `attach_shared.rs` and
  `attach_runtime.rs` run on Windows too. `attach/guard_tests.rs` (Unix):
  ownership, narrowing on one handle, the re-check after bind, the
  post-bind socket check and the path length. `attach/lock.rs`: the lock,
  a probe's shared lock refusing neither another probe nor, once
  released, a starting listener.
  `tests/attach_pipe.rs` (#2201, Windows): a client named as another user
  (by a stand-in for its token, since a test can't run as another user)
  or with no user found closed unanswered and audited with its SID and
  pid; the bridge refusing a pipe served by another user, or one owned
  by another user, writing it nothing; a second listener refused by the
  lock, and a listener starting again at once after the first stops;
  the listener's first and
  later instances owned by and granting only the test's user, with the
  medium label; the pipe named from the salt and the run's nonce, both
  owned by and open to the test's user alone, a new name each run, the
  nonce withdrawn at shutdown, a crashed TUI's nonce leading to "no TUI
  is listening", a pipe made under a name seen during an earlier run not
  keeping the next run from listening, `connect` reaching the new name,
  and a workspace everyone may list refused.
  `attach/salt_tests.rs` (every platform): the name needing both the salt
  and the workspace, the salt made once, racing makers agreeing on one,
  and a corrupt, linked, non-file or readable-by-others salt file refused
  and left alone, and on Unix the owner and mode rule on made-up owners;
  on Windows, the workspace check. `attach/nonce_tests.rs` (every
  platform): two runs naming the pipe differently, the nonce published
  private, replaced whole and withdrawn, a failing rename retried a few
  times, stale temporary files swept, and a corrupt, linked, non-file or
  readable-by-others nonce file refused. `attach/win32_tests.rs`
  (Windows): the owners and entries the salt file and the workspace
  accept, each forbidden right refused, and a workspace junction
  refused. `attach/pipe_tests.rs`: every instance's refusal of remote
  clients and the first's claim on the name, a name already held under
  a chosen nonce refused with no nonce published, and the kernel's copy of
  the descriptor and label.
  `fence_tests.rs`: unsafe code only in `attach/win32.rs`.
  `crates/bacnet-cli/src/tui/tests/mcp_attach.rs`:
  an agent on an open session's socket reading through its client, on the
  watch list's meter, `MCP n` and `MCP off` on the bar, the watch list's
  first SubscribeCOV keeping the first subscriber process identifier, and
  quitting ending it; `mcp_agent_pane.rs`: the agent pane's frames at
  120x40 and 80x24 (empty, with calls, with a waiting request's age, with
  dropped calls counted), an agent's read, confirmed write, declined write
  and unarmed refusal shown with their outcome and cost, the meter's
  agents' share matching the pane, disarming refusing the queue at once,
  and `bacnet://session` following the TUI's arming; `board.rs` in
  `bacnet-mcp`: the ring dropping its oldest, never waited on, and the
  outcome and target summaries;
  `crates/bacnet-cli/tests/mcp_connect.rs`: `bacnet mcp connect` against a
  listener, stdout JSON-RPC only, and the refusal with nothing listening.
