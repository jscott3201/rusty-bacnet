# MCP server (`bacnet-mcp`, `bacnet mcp serve`)

A [Model Context Protocol](https://modelcontextprotocol.io) server for
BACnet sites, read-only unless a profile offers writes: phase 1 (#2066, the
tools), phase 2 (#2067, resources and subscriptions) and phase 3 (#2068,
writes) of the MCP epic (#2064). The design, with
the later phases, is [design/mcp.md](design/mcp.md); the user guide is the
website's [AI agents](https://jscott3201.github.io/rusty-bacnet/guides/ai-agents/)
page. Experimental in 0.15.

## Pieces

| Piece | Where | What it does |
|---|---|---|
| `bacnet-mcp` | `crates/bacnet-mcp` | `McpServer<T>`: the tool surface, prompts, policy, untrusted-text envelope, signed cursors. Out of `default-members`; publishable, because the CLI depends on it. Default feature `stdio`. |
| Engine extensions | `bacnet-ops` | `RequestLimits::with_network_rates`, `turn_on` and `slot`; per-caller `Account` and `TokenBucket` through `limits::charged`; `Engine::who_is`, `discover_routers`, `devices`, `device`, `address_style`; `read_trend` charged per request; `alarm_summary` and `enrollment_summary` |
| Decoder | `bacnet-ops::decode` | The capture screen's frame summary and field tree, moved from the CLI for `bacnet_decode`: any tapped link (B/IP, B/IPv6, BACnet/SC, MS/TP), or a bare NPDU or APDU; a field quoting a character string is marked as device text |
| Watch hub | `bacnet-ops::watch` | `Hub`: one managed COV subscription or poller per point, shared by every listener, holding the latest values (phase 2); `Engine::device_events` |
| `bacnet mcp` | `crates/bacnet-cli/src/mcp.rs` | `serve --stdio [--profile NAME] [--passive]` and `authorize --profile NAME`, behind the CLI feature `mcp` (which brings `tui`, for the profiles). Classified `Read` in `bacnet_ops::class`. |
| `[mcp]` profile table | `crates/bacnet-cli/src/tui/profile/mcp.rs` | networks, devices, slow and fast networks, broad discovery, rates and caps; `mcp_writes`, `mcp_arm` and `[mcp.plan]` (#2068) |
| Write safety core | `bacnet-ops::safety` | The TUI's plan, ledger, audit log and workspace lock, and the engine half of a write (`desk`: pre-image reads, the stale check, the ledger and audit around each send, revert), shared by the TUI and MCP (#2068) |
| Writes | `crates/bacnet-mcp/src/writes/`, `tools/write.rs`, `tools/revert.rs` | The write session, the approvers (`ClientElicitation`, `PlanApprover`, `Deny`), the sealed confirmation state, and the two write tools |

The SDK is `rmcp` 3.5.1 (pinned, server role, stdio only). It answers both
the 2026-07-28 request form (`server/discover`, version and capabilities in
each request's `_meta`) and the older `initialize` handshake.

## Tools

The read tools are read-only (`readOnlyHint: true`, `destructiveHint:
false`); the two [write tools](#writes) are listed only when the profile
offers writes. All have input and output schemas and structured content
plus a text rendering.

| Tool | Notes |
|---|---|
| `bacnet_discover` | Who-Is (local, network, directed, global), then the discovery table, paged (pages index the live table, which can change between them). Broad (global, no range, or more than 1000 instances; directed without a range to an address that isn't a known device) needs `broad_discovery`, one a minute, whatever the reach; with `[mcp] devices` the range must also lie inside them. A send the transport refuses isn't charged. A network-scope Who-Is takes that network's slot. The one tool in passive mode, where it only lists (by default) and refuses to send. |
| `bacnet_read` | RPM with the engine's RP fallback; array elements by RP; `preset` `point` or `device`; at most `max_values` rows a call, then a cursor |
| `bacnet_list_objects` | Object_List, cached five minutes per device for paging; `types` filter; `names` reads each name (50 a page). A list read element by element stops at the call's budget and keeps its place, so the next call (with `next_cursor`) goes on from there; `list.complete` says when it's done. Calls on one device take turns, so progress never goes back. A resumed read checks the count again and starts over if it changed, and one begun more than 10 minutes ago starts over; an element the device can't give is a gap (`list.gaps`); at most 64 devices' lists are kept |
| `bacnet_describe_device` | The Device object's summary properties, services and object types by name, the address and route; `count_objects` |
| `bacnet_find_object` | Objects across the discovery table, within reach and an optional device `range`: by `name` or `description` (a case-insensitive substring of the cleaned device text), `object` or `types`. Lists come from `bacnet_list_objects`' cache (read once, kept five minutes); a text match reads each candidate's name (and description, if asked) once, kept five minutes too. A page holds up to 100 matches (or the policy's object page), and a full page ends before the next device is read; the cursor packs a device and a candidate index. Names are kept cleaned and capped at 4,096 characters, which a search matches against (a result shows the policy's text cap of them), at most 4,096 objects a device and 16,384 in all. A device whose list read stopped is under `incomplete` with why, after searching what was read; one that failed is under `skipped` with its error; a list read cut by the budget resumes on the next page. A device that stops answering while its names are read is under `incomplete` too, and the search moves on without a cursor back to it, so a silent device doesn't cost each later page a timeout; search it alone to retry. |
| `bacnet_network_map` | Networks in `[mcp] networks`, each with its routers (from the last Who-Is-Router-To-Network and from the routed devices' router addresses), its speed and rate under the policy, and its devices in reach; and the routers with the networks each reaches. `send: true` broadcasts a Who-Is-Router-To-Network: it needs `broad_discovery`, goes at most once a minute (checked and claimed under one lock), takes the local network's slot and is charged. The wait for answers ends with the call's budget, counted after the wait for the rate slot. A query cancelled while it waits (for the slot or for answers) keeps its claim on the minute but records no routers; at most 256 routers are kept, replaced by each finished query (one asking about a single network merges into them, the routers just heard first). `bbmd: true` adds the BBMD tables and a consistency verdict (see [BBMD tables](#bbmd-tables)). No network-layer writes, table writes or registrations. |
| `bacnet_read_trend` | A Trend Log or Trend Log Multiple through `Engine::read_trend`, each request charged. From the oldest record or after `from`, up to `to`; at most `max_records` (500 by default, 2,000 at most, and about 60 kB of structured content) a call, then a cursor holding the log's next sequence number (a log that has more but gives no sequence number to resume from returns `truncated` without a cursor, and says why under `stopped`). `from` after `to` is a JSON-RPC -32602.
 Records: time, sequence number, `values` as `Datum`s (a failure as its error), status flags named, `flagged` when a status flag is set or the record is a log status change. A summary of the records read comes with every call: count, flagged, log status and time-change counts, first and last time, and per column (a member of a Trend Log Multiple) count, min, max and mean of numbers, counts of booleans and enumerations by value, failures. Once the device has answered the first page, a call reads the log's Log_DeviceObjectProperty once (one more charged request, through `Engine::log_columns`, so a silent device still costs one timeout) and names each column's enumerations and bit strings by the property it samples, in `values` and in the state counts (#2091); when that read fails or names nothing, values keep their numbers. State keys are therefore names when the reference names them and numbers otherwise, and can differ between calls on the same log if the reference read fails on one of them. A column that samples a binary or multi-state object's state (its Present_Value and the like) counts its samples as states, keyed `active`/`inactive` or by state number, never as numbers to average; when the reference read fails, the call doesn't know the column holds states, so a multi-state column falls back to `numeric` and `numeric` versus `states` can also differ between calls. The call then reads that object's Active_Text and Inactive_Text, or State_Text: one more charged request an object, once a call, for at most 8 objects on the log's own device, and skipped once the call's budget is spent (#2122). The texts are device strings, so they never become keys or join a value: the column's `state_texts` maps each state key it counted to its text, as the device sent it, in an untrusted envelope (cleaned and capped there, with `truncated` set when cut) (source `device N multi-state-value:1 state-text`, `active-text` or `inactive-text`), and is null when no key has one (a refused read, an empty text, a state past the end of State_Text). `summary: true` reads up to 20,000 records and returns only the summary. `pages`, `more_items` (answers cut short by the device) and `gaps` report how the reading went. |
| `bacnet_alarms` | GetEventInformation through `bacnet_ops::format::alarms` (#2077): objects, states, transitions, notify types, timestamps and priorities named; paged over the device's answers (`more_events`) up to `max` (100) a call, the cursor holding the last object; once the call's budget is spent no further answer is asked for, and what was read comes back with `more_events` and a cursor (`format::alarms::read_event_information_until`).  `summaries: true` adds GetAlarmSummary and GetEnrollmentSummary on the first page; a device refusing one reports it under `summary_errors`. When the events spent the call's budget, neither is sent, and each shows under `summary_errors` as `{"skipped": "budget"}`. Every datum is passed through the envelope check. |
| `bacnet_watch` | Points on the watch hub, and what changed since the last call (see [the tool-only fallback](#the-tool-only-fallback)). `idempotentHint: false`. Not listed in passive mode. |
| `bacnet_decode` | Hex (separators and `0x` ignored, at most 6,144 octets, room for the largest BACnet/SC message the stack accepts) as `bip`, `bip6`, `mstp`, `sc`, `npdu`, `apdu` or `auto` (by the first octets; SC must be named). Returns a one-line `text`, the decoder's `summary` and the field `tree` (labels with `[start, end)` octet ranges, at most 600 fields). A label quoting a character string is an envelope with source `decoded frame`; other labels are cleaned, and a long one (an unknown tag's octets) is cut at 512 characters with `truncated: true`. No traffic: listed in passive mode too. |

Prompts: `survey_site` (range, network) and `investigate_point` (device,
object). Their arguments are checked against the target grammar. The survey
also maps the networks and lists each device's alarms; the point
investigation also checks its alarms and summarises a trend log of it.

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
| `bacnet://device/{instance}/object/{type}:{n}` | the point facts (`bacnet_read`'s `preset: "point"` properties); `source` `read` when read just now, `cov` or `poll` when watched, with `state` (`starting`, `live`, `failed`, `unreachable`, `stopped`), `note` (why it is polled or not live), `seq`, `age_ms` and `updates` (notifications, coalesced, polls, renewals) | a watch on the hub |
| `.../property/{prop}` | one property (`name` or `name[index]`; `[`, `]` and `:` may be percent-encoded) | a property watch (SubscribeCOVProperty) |
| `bacnet://alarms` | each device in reach: its first page of GetEventInformation, as far as the read's budget goes (`unread` past it, `errors` per device) | a poll every 60 s; an update when the events change |
| `bacnet://device/{instance}/alarms` | one device's first page of events | the same, for one device |
| `bacnet://session` | level (`read-only`, `write-session` while a session or plan grant is in force, `write-session (expired)`), `writes` (`not offered`, or the mode, arming and what the client may confirm), `expires` (seconds left in an armed session), `grants` (a plan grant: in force, used, budget, expiry), limits, the request meter, the principal's watches and the hub's footprint (points, subscribed, polled, requests, lagged notifications, lifetime) | honoured; no updates yet (#2068 left the notifications on arming and settling for later) |
| `bacnet://ledger` | this principal's ledger entries in effect (`ledger_id`, target, value written, what undoes it, state, time), read from the profile workspace's `ledger.json`; empty without writes | honoured; no updates yet |

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

`rmcp` 3.5.1 implements `subscriptions/listen` (2026-07-28): the server
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
  point cap). `resources/unsubscribe` drops the subscription and its watch
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
  still cancelling).
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
  with an optional `[index]`, `value`, `priority`), `batch`
  (WritePropertyMultiple; several writes need it) and `reason`. Targets are
  numeric (`ao:1`, never a name); a value is a number, a boolean, `null`
  (relinquish), a typed `{"value": n}` or the CLI value grammar, checked
  before anything is sent. The result names each write's `outcome` (the
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
  labelled) and a sealed `requestState`. A client that didn't declare form
  elicitation, or an older-revision session, is refused
  (`elicitation-unsupported`); the older server-initiated elicitation is
  not used yet.
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
  leaves the entry `unknown`. The lock is released after.
- **`bacnet_revert`**: `entries` (ledger ids) or `all`, this principal's
  MCP entries only (`not-yours` for another's), on devices the policy
  reaches; no prompt and no armed session, as in the TUI. Latest write
  first, through the desk; the result names each entry's outcome and how
  many are `left`.
- **Plan grants** (`[mcp.plan]`: `devices`, `properties`, `priority_min`
  and `priority_max` (8 and 16 by default), `max_writes`, `hours` (1 to
  8), `reason`) run from `bacnet mcp authorize`, which spells out what the
  plan lets through without a prompt; the budget counts per process.
  Offering writes, letting the client arm, or any change to a plan clears
  the authorization.
- **Audit.** Every MCP record has `front_end: "mcp"`, `principal`,
  `client` (`claimed_name`, a claim), `tool`, `args_digest` and
  `request_id`, and a write `confirmed_via` (`elicitation` or `plan`) and
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
  characters by default).
- Cursors are `hex(body).hex(HMAC-SHA256)` under a key drawn per process,
  binding the tool, a digest of the principal (the OS user's name doesn't
  show), a digest of the other arguments, an expiry (10 minutes) and the
  position.

## Limits

Every request goes through `RequestLimits`: the caller's token first (5 a
second, burst 20), then the device's gate (one outstanding request), then
its network's rate (20, or 2 for `slow_networks` and 5 for
`fast_networks`; a router doesn't say what link it reaches), then the
global rate (20). Each tool call runs on its own task under an `Account`
that counts its requests and waits. A call that pages stops at its budget
(30 s) and returns a cursor; one still waiting `budget + 15 s` later
returns `refused: rate-limited` if it sent nothing, else `timeout`.

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

## Tests

- `crates/bacnet-mcp/src/tests/`: the stdio protocol over a pipe against
  an in-process lab (handshake, tool list, survey, errors, cursors, the
  2026-07-28 request form), no write tool in any configuration, passive
  mode, 500 reads within the global cap, one request outstanding per slow
  device, a slow network held to 2 a second, the injection fixture, the
  policy's reach, broad discovery (wide ranges, ranges outside the policy,
  parallel calls), network numbers, network-paced Who-Is, cancellation, a
  long indexed Object_List across calls, and output schemas (success and
  error results of all ten tools, nested values included).
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
