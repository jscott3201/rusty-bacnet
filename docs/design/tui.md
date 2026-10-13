# Design: a terminal UI for the `bacnet` CLI

Status: accepted (2026-10-01). Milestone M0's first issue, the scaffold and the
live devices screen (#961), is implemented; the rest is planned work tracked by
epic #975. Spec references are clause numbers in ANSI/ASHRAE 135-2020,
paraphrased. Anything marked **planned** does not exist yet.

## 1. Summary

`bacnet` is a clap one-shot CLI plus a rustyline shell (`crates/bacnet-cli`).
This design adds a full-screen terminal UI, `bacnet tui`, built on ratatui 0.30
and crossterm 0.29. It is for people who commission, service and debug BACnet
networks, and for people building on rusty-bacnet. The one-shot commands stay
as the scriptable, JSON-producing and screen-reader-friendly interface.

The transports that matter most are BACnet/IP (including BBMD and
foreign-device operation), BACnet/SC (hub connection, failover, TLS
certificates) and MS/TP (RS-485). A keyboard-driven tool that covers all three
on Linux, macOS and Windows, with diagnostics built in, is useful in its own
right, and a TUI also runs over SSH on a box that is already on site, such as a
Raspberry Pi with a USB-RS485 adapter or a jump host inside an OT VLAN.

## 2. Goals and non-goals

**Goals**

- One `bacnet` binary on Linux, macOS and Windows that opens a TUI for
  discovery, browsing, guarded writes, live watching and transport diagnostics.
- Diagnostics per transport: BBMD and foreign-device health for B/IP, a
  connection doctor and certificate lint for SC, and a passive monitor with
  token-ring analysis for MS/TP.
- Read-only by default. Changes go through an explicit, time-boxed write
  session with previews, a revert ledger and an audit log.
- The one-shot CLI contract does not change: subcommands, aliases, flags,
  automatic JSON when piped, JSON and NDJSON shapes, errors on stderr with an
  empty stdout, and the SC error phases.
- The TUI uses public stack APIs only. Where the stack lacks something, the gap
  is filed and fixed in the stack rather than worked around in the CLI.

**Non-goals**

- A GUI, a web UI, or a replacement for Wireshark's decoder.
- Hosting a production server, router or SC hub from the TUI. A local lab hub
  is a later item.
- Generating credentials (a lab PKI).
- Scripting or macros, BACnet Ethernet, BACnet/IPv6-specific screens, COBS
  transmit, backup and restore, schedule and event-enrollment editing.
- Python parity: the wheel has no CLI entry point.

## 3. Users and workflows

Personas: commissioning technicians, service technicians, integrators, network
and IT/OT engineers, operators, and rusty-bacnet developers.

| ID | Workflow | Transports | Priority | Issue |
|---|---|---|---|---|
| W1 | What is on this network? Who-Is ranges, live I-Am table, duplicate instances; Who-Has; routers and networks | all | P0 | #961 (done), #960 |
| W2 | Why can't I see device X? Interface and port, BBMD/FD, router path, SC handshake phase, MS/TP token | all | P0 | #962, #966, #967, #970 |
| W3 | Point checkout: browse, read values and status flags, override at a priority, release | all | P0 | #963, #964 |
| W4 | Watch live values (COV with renewal, polling fallback) | all | P0 | #965 |
| W5 | BBMD and foreign-device health: BDT consistency, FDT, TTL, NAT view | B/IP | P0 | #966 |
| W6 | SC onboarding: connect, verify certificates, diagnose TLS failures, primary and failover state | SC | P0 | #967, #968 |
| W7 | MS/TP trunk health: stations, token rotation, Max_Master gaps, errors; join safely | MS/TP | P0 | #969, #970 |
| W8 | Capture and decode traffic, filter, export pcap | all | P0 | #971 |
| W9 | Alarm summary and acknowledgement | all | P1 | follow-up |
| W10 | Trend log and schedule review | all | P1 | follow-up |
| W11 | Device admin: time sync, DCC, reinitialize, file read | all | P1 | follow-up |
| W12 | Lab: local SC hub, simulated devices | all | P1 | follow-up |
| W13 | Backup and restore, file write, create and delete object, schedule editing | all | P2 | later |

Alarm viewing is P1 because the client still returns GetEventInformation as raw
bytes even though `GetEventInformationAck::decode` exists; wiring the decoder in
is a small follow-up.

## 4. UX

### 4.1 Layout

The target layout once the later screens exist:

```
+ bacnet | site: plant-a | BIP 10.0.1.5:47808 | FD -> 10.0.0.1 (212 s) | READ-ONLY | COV 3 | drop 0 +
| 1 Devices  2 Browse  3 Watch  4 Network  5 SC  6 MS/TP  7 Capture                              |
+----------------------+------------------------------+--------------------------------------------+
| Devices (42)         | Objects: Device 1234 (318)   | AO:1 "Supply Fan Speed"                    |
| > 1234 10.0.1.20     |   analog-input (40)          |   present-value   42.0 %                   |
|   1301 2:0x05        | > analog-output (12)         |   status-flags    [OVR]                    |
+----------------------+------------------------------+--------------------------------------------+
| log: 12:01:03 INFO COV renewed AO:1 on 1234 (lifetime 300 s)                                    |
+ ? help  / filter  : command  W arm writes  u ledger  q quit -----------------------------------+
```

What M0 draws (80x24):

```
 bacnet | BIP 10.0.0.1:47808 | up | READ-ONLY | drop 0                   ? help
┌ Devices (5) | sort: instance ↑ ──────────────────────────────────────────────┐
│  Instance* Address                         Net    Vendor APDU  Seg      Seen │
│> 1001      10.0.1.21:47808                 local  260    1476  none     2s   │
│  2001      05 via 10.0.1.1:47808           2      24     480   both     7s   │
└ Who-Is local 1000-2999: done, 3 replies ─────────────────────────────────────┘
 d Who-Is  / filter  s sort  S reverse  L log  ? help  q quit
```

- The status bar shows the transport and local address, link state, the safety
  level, and a dropped-events counter. Later milestones add the site, the
  foreign-device countdown or SC hub state, and the remote footprint (active
  COV subscriptions and FD registrations).
- Screens appear only where they apply: Network for B/IP, SC for SC, MS/TP for
  a serial port. Devices, Browse, Watch and Capture are always present.
- 80x24 is the functional minimum and 120x40 the full layout. From 120 columns
  the device table uses its wide columns; below 120 the planned Browse panes
  stack and the focused pane fills the width. Below 80x24 the TUI shows a
  "terminal too small" notice and nothing else.

### 4.2 Screens

| Screen | Contents |
|---|---|
| Devices (M0) | Who-Is form (local, global, directed, remote network; instance range 0 to 4194303); live table (instance, address, network, vendor, max APDU, segmentation, last seen); sort and filter; duplicate-instance banner. **Planned:** Who-Has search and a Networks tab |
| Browse (planned) | Device card (vendor, model, firmware, protocol revision, services and objects supported, Max_APDU, segmentation, Database_Revision); Object_List grouped by type; property pane; priority-array pane |
| Watch (planned) | Watched points with value, flags, COV or polled, last-update age, sparkline |
| Network, B/IP (planned) | Foreign-device panel, BDT, FDT, BDT consistency matrix, "what the BBMD sees me as" |
| SC (planned) | Hub connector status, connection-doctor phase timeline, local and peer certificates, VMAC and UUID |
| MS/TP (planned) | Participant panel or passive monitor (station grid, token timing, Max_Master analysis, error counters, join wizard) |
| Capture (planned) | Frame list, decode tree, hex view, filters, pcap export |
| Overlays | Help (`?`), log pane (`L`) and interface picker in M0; session ledger, confirmation modals and profile picker later |

### 4.3 Keybindings

M0 keys:

| Key | Context | Action |
|---|---|---|
| `?` | global | show or hide help |
| `q` | global (not in text fields) | quit |
| `Ctrl-C` | global | close the open dialog or cancel the running Who-Is; a second press within 2 s quits |
| `L` | global | show or hide the log pane |
| `d` | Devices | open the Who-Is form |
| `/` | Devices | filter rows (substring of instance, address, network or vendor); Enter keeps it, Esc clears it |
| `s` / `S` | Devices | next sort column / reverse the order |
| arrows, `j`/`k`, `PgUp`/`PgDn`, `Home`/`End`, `g`/`G` | Devices | move |
| `Tab`, `Shift-Tab`, Up, Down | Who-Is form | next or previous field |
| Left, Right, Space | Who-Is form | change the scope |
| `Enter` | Who-Is form | send; a global or unbounded request needs a second Enter |
| `Esc` | dialogs | close |

Planned: `1` to `7` and `Tab` to switch screens, `:` for a command bar that
accepts one-shot syntax, `W` to arm writes, `u` for the ledger, `r` to refresh,
`a`, `w`, `n`, `x`, `Space`, `e`, `c` and `J` on later screens.

Only key presses count; Windows also reports releases. Mouse capture will be
opt-in (`--mouse`) because it breaks native text selection.

### 4.4 Accessibility

Full-screen redraws are close to unusable with screen readers, and ratatui has
no accessibility layer. Mitigations:

- Every TUI action has a one-shot or JSON equivalent. That is the supported
  accessible path.
- State is never shown by colour alone. Badges and markers are text
  (`READ-ONLY`, `DUPLICATE`, `>` for the selected row, `*` on the sorted
  column; later `[ALM]`, `[FLT]`, `[OVR]`, `[OOS]`). A snapshot test checks
  that the coloured and `NO_COLOR` frames carry the same text.
- `NO_COLOR` is respected: styles fall back to bold, dim and reverse video.
- `--fps` lowers the frame rate.

## 5. Safety model

The project README asks for authorization before use on a network. Today the
one-shot CLI asks for no confirmation before writes, DCC or reinitialize; #972
tracks guarding those separately, without changing the one-shot contract here.

**Session levels** (always shown in the status bar):

1. **Passive** (planned, `--passive`): never transmits. Capture, the MS/TP
   monitor and offline files only.
2. **Read-only** (the default, and the only level in M0): discovery, reads,
   ReadRange, BDT and FDT reads. COV subscriptions and FD registrations create
   state on remote devices, so they always use finite lifetimes, are cancelled
   on exit, and are counted in the footprint.
3. **Write session** (planned, #964): armed with `W`. A modal shows the site and
   network, asks for an optional reason or ticket, and sets a duration (default
   15 min). The bar turns red with a countdown and the session disarms when it
   expires. A profile can set `writes = "never"`.
4. **Dangerous operations** (planned): off unless the profile enables them and a
   write session is armed.

**Confirmation tiers**

| Tier | Actions | UX |
|---|---|---|
| T0 | Reads, discovery, capture | None, except that a global or full-range Who-Is gets a one-line warning with the expected response volume and needs a second Enter (M0) |
| T1 | Writes at priorities 9 to 16, non-commandable property writes | Preview (old to new, slot occupancy, whether the effective value changes), then Enter |
| T2 | Priority 8 overrides, Out_Of_Service toggles, WPM batches, relinquishing a slot we did not write, alarm ack, time sync, MS/TP join | Preview plus `y`; recorded in the ledger with the pre-image |
| T3 | Priorities 1, 2 and 5, DCC, reinitialize, Write-BDT, Delete-FDT-Entry, create and delete object, file write | Shows the blast radius; the user types the device instance (or BBMD address) to confirm. DCC defaults to a finite duration |
| Blocked | Priority 6, which the standard sets aside for minimum on and off times (Clause 19.2.3); the deprecated plain-disable DCC option, which current-revision servers ignore (Clause 16.1) | Expert flag only |

Before any commandable write, the TUI will read the priority array. It warns
when a higher-priority slot is active (so the write would not change the present
value) and when the target slot already holds another writer's value: writing a
slot replaces whatever another party put there, and that party is not told
(Clause 19.2).

**Session ledger** (planned). Every change records its pre-image (slot value,
Out_Of_Service, property value, BDT). The ledger is written to the workspace on
each change, so it survives a crash. Revert-all relinquishes our slots, restores
Out_Of_Service and rewrites old values. Quitting with outstanding overrides
opens a modal: revert all, keep and log, or cancel. The next launch on the same
profile lists entries left behind by a crashed session.

**Audit log** (planned). `audit.jsonl` in the profile workspace is append-only.
Each record holds UTC time, OS user, host, profile, transport and local
address, target, service, decoded parameters, priority, pre-image, outcome
(ack, error class and code, reject, abort, timeout) and session level.
Passwords are redacted, file payloads are reduced to size plus SHA-256, and key
material never appears. Arming, disarming and declined confirmations are
recorded too. This is a client-side record, separate from BACnet audit
reporting (Clause 19.6), and it complements the server-side
[mutation policy](../mutation-policy.md); SERVICE_REQUEST_DENIED is shown
distinctly.

**Other rules**

- Passwords for DCC and reinitialize are entered in masked fields and never
  stored, in history or anywhere else.
- Polling uses conservative defaults: a minimum interval per point, one
  outstanding confirmed request per device while browsing, and a global
  request-rate cap.
- An active MS/TP join is T2: joining a trunk means taking part in token
  passing (Clause 9.5), so a duplicate or out-of-range MAC affects every device
  on it.
- The first use of a profile records an authorization attestation with a
  timestamp.

## 6. Architecture

### 6.1 Entry points and the one-shot contract

- `bacnet tui` opens the TUI. With no TTY on stdin or stdout, or with
  `TERM=dumb`, it exits non-zero with a hint on stderr and writes nothing to
  stdout, so `bacnet tui > out.txt` leaves `out.txt` empty.
- A bare `bacnet` and `bacnet shell` still open the rustyline shell. Switching a
  bare `bacnet` to the TUI, with `--no-tui` and `BACNET_NO_TUI` as escape
  hatches, is #974.
- The global transport flags (`-i`, `-p`, `-b`, `-t`, `--ipv6`, `--sc*`) feed
  both modes. Profiles (§6.6, planned) add defaults that explicit flags
  override.
- The one-shot contract that must not change: subcommand names and aliases;
  global flags on either side of the subcommand; automatic JSON when not a TTY;
  JSON and NDJSON shapes with display-string values; errors on stderr with an
  empty stdout and a non-zero exit; stable `--sc-*` error phases; feature-off
  rebuild advice. Consumers include `scripts/release/cli_smoke.sh`, the README,
  the website guides and the integration tests in `crates/bacnet-cli/tests/`.
- The shell's hand-written argument parsers have drifted from clap. The planned
  command bar reuses shared parsing (`src/core/`) instead of adding a third
  parser.

### 6.2 Crate and module layout

Everything stays in `crates/bacnet-cli`, behind the `tui` cargo feature
(opt-in until the TUI ships in 0.14.0, #975), and outside the workspace's `default-members` as before. A separate
crate is not justified until a second consumer exists. Building without
the feature drops ratatui and crossterm; `bacnet tui` then prints rebuild
advice and exits 1.

```
crates/bacnet-cli/src/
  main.rs            dispatch: one-shot | shell | tui (the hand-built runtime from #950)
  args.rs            + the Tui subcommand (--fps, --log-file)
  core/              code shared by one-shot, shell and TUI: interface listing,
                     Who-Is range parsing (more as later screens need it)
  commands/          one-shot handlers (contract unchanged)
  tui/
    mod.rs           entry: TTY guard, tracing, interface choice, run and teardown
    terminal.rs      raw mode and alternate screen, panic hook, signals
    app/             App, Action, update() -> Vec<Command>; device table, Who-Is form, picker
    keymap.rs        keys to actions per context
    event_loop.rs    select! over input, frames, worker events and signals
    message.rs       Command and WorkerEvent: the UI <-> worker protocol
    log_layer.rs     tracing layer -> ring buffer
    worker/          generic over T: TransportPort; owns the BACnetClient
    view/            pure rendering: devices table, chrome, dialogs
    tests/           app-core, frames (insta), in-process lab, load, event loop
```

Planned additions: `safety.rs` (levels, tiers, ledger, audit writer),
`profile.rs` (TOML profiles and workspaces), and one worker and view module per
later screen.

The 700-line file cap (`scripts/ci/check-file-size.sh`) and `missing_docs =
deny` apply. The TUI writes through the crossterm backend, not `print!`, so the
workspace `print_stdout` lint is not involved.

### 6.3 Async model

```
crossterm EventStream ----+
frame interval (20 fps) --+--> UI loop on the block_on thread
worker events (bounded) --+      update(&mut App, Action) -> Vec<Command>
signals ------------------+      if dirty: terminal.draw(view)
                                       |
                                       v
                          bounded command channel -> worker task (tokio::spawn)
                                                       uses BACnetClient<T>
```

- **UI loop.** It runs inside the boxed `cli_main` future that the hand-built
  runtime `block_on`s (#950), not in `tokio::spawn`, so a slow `terminal.draw`
  never stalls a worker. Each pass is a `select!` over the crossterm
  `EventStream` (key presses only), a frame interval (default 20 fps, set with
  `--fps`), the bounded worker-event channel, and the termination signals.
- **Redraws.** The state carries a dirty flag. A key press that changes it
  redraws straight away, so typing feels immediate; worker events only mark it
  dirty and are drawn at the next frame tick, so a burst costs one frame however
  many events it holds. Ages in the table tick once a second, so an idle screen
  is not redrawn at all. Worker events are applied up to 256 per wake-up before
  input and frames get a turn.
- **Worker.** One task owns the client. It takes bounded `Command`s and emits
  `WorkerEvent`s; operations carry an id, and `update` drops results for an
  operation that is no longer current. The UI closes the command channel to
  stop it, and the worker stops the client.
- **High-rate streams.** The client's device broadcast is folded into the
  bounded worker-to-UI channel (1,024 events). When the UI falls behind, events
  are dropped and counted, and a lagging broadcast receiver adds its count; the
  status bar shows the total. After any loss the worker resynchronises the table
  from the client's own discovery table, at most once a second. Duplicate-
  instance notices are rare and important, so they wait for room instead of
  being dropped; the record behind the banner keeps at most four addresses per
  instance and counts the rest, and its text is rebuilt only when it changes.
  Lost devices leave the table with a line in the log pane. Later streams
  (COV, events, frames) get the same treatment.
- **Latest-value state** (SC link state, MS/TP statistics, FD countdown) will
  use `watch` channels.
- **MS/TP** will run with `MstpExecutionMode::DedicatedThread`, so terminal
  stalls cannot reach the MAC loop; Clause 9.5.3 timing works in tens of
  milliseconds.
- **Stack size** (#953). Windows gives the main thread 1 MiB, and debug futures
  are large. The TUI future is boxed, `App` is boxed, the worker and each
  client build are boxed futures on runtime threads, and the draw path is
  small. The CLI tests run under the native jobs' 1 MiB stack guard.
- **Cancellation.** In raw mode Ctrl-C arrives as a key, not SIGINT, so
  cancellation is key-driven: Ctrl-C closes the open dialog or cancels the
  running Who-Is, and a second press within two seconds quits.
- **Terminal ownership.** In TUI mode tracing goes to an in-app ring buffer
  (1,000 lines, shown in the `L` pane) and optionally to `--log-file`. Nothing
  writes to stdout or stderr while raw mode is on; that is why the BIP
  interface picker is a dialog rather than the shell's stderr prompt.
- **Teardown.** Setup mirrors `ratatui::init` and teardown calls
  `ratatui::try_restore`. The panic hook is our own so that it shares one
  idempotent restore with normal exit and signal exit and so that its order is
  testable: it restores the terminal first, then runs the previous hook, which
  prints the panic on a usable screen.
- **Panics off the UI thread.** The hook is process-wide, so a panic in the
  worker or in any client or transport task also restores the terminal, and
  tokio then catches it. The loop checks `terminal::is_active()` before every
  pass and every draw, so it never paints over the restored screen; it quits
  with an internal error and exit status 1. The worker's channel closing while
  the UI still runs is treated the same way.
- **Signals and exit status.** SIGTERM, SIGHUP and an external SIGINT (Unix),
  and console close and Ctrl-Break (Windows), end the loop and restore the
  terminal. On Unix the TUI then exits with 128 plus the signal number, as
  shells report it (143, 129, 130); on Windows it exits 1. The handlers stay
  installed while the worker shuts down, so a Ctrl-C during that wait (SIGINT
  again, now that the terminal is cooked) exits at once instead of being
  swallowed. Windows ends a process soon after console close, so that restore
  is best effort. A terminal that passes the TTY check but refuses raw mode
  (MSYS and mintty) gets the same hint as the TTY check and exit status 1.

### 6.4 State model

TEA-style: one `App` struct, one `Action` enum, and an `update` function with no
I/O that returns `Vec<Command>`. Time reaches `update` only through
`Action::Tick`. Views are pure functions of `&App` (the table keeps its scroll
offset in a `Cell` between frames), and the keymap turns keys into `Action`s.
Components do not get their own channels or locks, which keeps every state
transition testable without a terminal.

```
App { transport, link, devices: DeviceTable, duplicates, overlay, op, flash,
      now, dropped, log: LogRing, quit, exit_error, dirty, .. }
Action = Tick { now, dropped, log_generation } | Resize | Quit | CtrlC | Escape
       | ToggleHelp | ToggleLog | OpenWhoIs | Move | CycleSort | ReverseSort
       | StartFilter | Filter | Form | Picker | Worker(WorkerEvent)
Command = Connect { interface, broadcast } | WhoIs { op, spec } | Cancel { op }
```

Tables keep sorting and filtering in the model and render only the visible
rows, which matters for devices with up to 10,000 objects; drawing the device
table at 120x40 with 4,096 rows must take under 5 ms in a release build.

### 6.5 How the TUI talks to the stack

The TUI uses only public `bacnet-client` and `bacnet-transport` APIs.

- **Generics.** The worker is generic over `T: TransportPort` and instantiated
  per transport, as the one-shot commands are. `App` and the views are not
  generic, so monomorphization stays in the worker layer.
- **APIs used by M0:** `who_is`, `who_is_directed`, `who_is_network` and
  `broadcast_unconfirmed` (for a local-only Who-Is), `device_events()`,
  `device_collision_events()` and `discovered_devices()`.
- **APIs later screens will use:** RP and RPM, WP and WPM (local and routed);
  `subscribe_cov`, `manage_cov_subscription` and `cov_notifications()`; the BBMD
  helpers on any `BACnetClient` whose transport is `AsBip` (`BipTransport`, or
  `AnyTransport` with a `Bip` variant); `ScConnectError` and
  `ScWebSocketErrorKind`; the MS/TP `diagnostics()` handle and
  `decode_frame_stream`.
- **Transport handles.** `client.transport()` (#956) borrows the transport of
  any built client. Workers take the owned handles once (the SC
  `connection_state_changes()` watch, the MS/TP `diagnostics()` handle, the
  BBMD state `Arc`) and poll the counter snapshots (SC `npdu_drop_counts()`,
  B/IP management, FDT and fanout counters) through the borrow. They never
  send through the transport or hold the BBMD state lock across an await.
- **Monitor and capture.** The MS/TP passive monitor has no client: a worker
  owns the serial port (#958). Live capture of our own traffic needs a
  link-level tap (#957).

### 6.6 Profiles and workspaces (planned, #962)

- Profiles are TOML files in the platform config directory (via
  `directories::ProjectDirs`), overridable with `BACNET_CONFIG_DIR`.
- One profile per site, holding transport settings (interface, port,
  broadcast, BBMD and TTL; SC primary and failover URLs and **paths** to the
  CA, certificate and key; VMAC; a device UUID generated once and persisted,
  since Annex AB.1.5 expects it to stay the same for the device's life; MS/TP
  port, baud, MAC, Max_Master and Max_Info_Frames) and a safety policy
  (`writes = "never" | "session"`, `dangerous = false`).
- A workspace directory per profile holds the device cache (keyed by
  Database_Revision), watch lists, captures, the ledger and `audit.jsonl`.
- Passwords and key bytes are never stored. Flags-only runs work without a
  profile. The shell history moves off `$HOME`, which is unset on Windows.

## 7. Crate choices

| Crate | Version | MSRV | Licence | Use |
|---|---|---|---|---|
| ratatui | 0.30.2 | 1.88 | MIT | core; default features minus the calendar widget (which pulls in `time`) and macros |
| crossterm | 0.29 | 1.63 | MIT | backend with `event-stream`, imported as `ratatui::crossterm` |
| futures-util | workspace 0.3 | | MIT/Apache-2.0 | `StreamExt` on `EventStream` |
| own tracing layer | | | | the log pane (a small `Layer`; avoids `log` and `env_filter` through tui-logger) |
| dev: insta | 1.48 | 1.66 | Apache-2.0 | snapshot tests |
| dev: tokio `test-util` | workspace | | MIT | paused-time tests |
| later, if needed | tui-tree-widget 0.24 (capture decode tree), tui-input 0.15 (command bar), toml 1.1 and directories 6.0 (profiles), x509-parser 0.18 behind `sc-tls` (certificate lint), tui-popup, tui-scrollview, nucleo-matcher (MPL-2.0, allowed by `deny.toml`), portable-pty and vt100 (PTY end-to-end tests) | | | |
| skipped | color-eyre, better-panic, human-panic, template config stacks | | | the panic hook above already restores the terminal |

- **MSRV.** The highest requirement is 1.88 (ratatui), against a workspace MSRV
  of 1.93. ratatui has raised its MSRV in a patch release before, so the
  workspace uses `resolver = "3"`, which prefers dependency versions compatible
  with the declared `rust-version` when the lock file is updated. The committed
  lock file, `--locked`, and the MSRV CI job remain the backstop.
- **Size.** Measured for #961 on Linux x86_64 release builds, the `tui` feature
  adds about 1.0 MB to the default `bacnet` (7.20 to 8.20 MB; 5.40 to 6.13 MB
  stripped) and 1.1 MB with `sc-tls`, as the release assets are built (13.72 to
  14.84 MB; 10.84 to 11.68 MB stripped). The workspace has no
  `[profile.release]` tuning (strip, LTO) yet; #973 measures it before
  deciding.
- **Duplicates.** The only new semver-incompatible duplicate in the built
  graph is `hashbrown` 0.16 (via `kasuari`). `Cargo.lock` also gains entries for
  ratatui's optional backends (termwiz, termina, with `thiserror` 1), which are
  never built.

## 8. Testing strategy

| Layer | What | Where it runs |
|---|---|---|
| a. App core | `update()` driven by synthetic keys and worker events; no terminal | all three native CI OSes |
| b. Frames | `TestBackend` frames at 80x24 and 120x40 as insta snapshots; a check that colour never carries information the text lacks | all OSes |
| c. In-process loopback | real `BACnetServer`s and a `BACnetClient` on an in-memory hub transport, driven through the worker and `update`: range and unbounded Who-Is, duplicate instances. Later: BBMD-mode BIP, a `BACnetRouter` between two networks, an mTLS `ScHub` | all OSes |
| d. Event loop | the `select!` loop on a `TestBackend` with scripted input and paused time: redraw-only-when-dirty, presses only, Ctrl-C, end of input | all OSes |
| e. Load | 1,000 device events per second against a stalled UI: the channel stays bounded and every event is delivered or counted; render time of a 4,096-row table | all OSes (the 5 ms budget is asserted in release builds) |
| f. Contract | the existing one-shot integration tests, unchanged; `bacnet tui > out.txt` fails with a hint and an empty file | all OSes |
| g. Docker topology | `examples/docker/docker-compose.yml` (bbmd-a/b, router, sc-hub, foreign-client) | manual and release checks |
| h. MS/TP (planned) | `LoopbackSerial::pair()`, PTY `SerialStream::pair()`, the ring simulator from #958; real USB-RS485 adapters for hardware checks | Unix for PTY; loopback everywhere |
| i. PTY end-to-end (planned) | start, quit, terminal restored, with portable-pty and vt100 | Unix first |

Rules: fixed terminal sizes; times as fixed offsets so ages render the same on
every run; addresses from the in-memory hub rather than ephemeral ports.

**Running the snapshot tests.** The tests live in
`crates/bacnet-cli/src/tui/tests/` and the accepted frames in its `snapshots/`
directory. CI and a plain `cargo nextest run -p bacnet-cli` compare against the
committed `.snap` files and fail on any difference. After an intended UI
change, regenerate and review them with
[cargo-insta](https://insta.rs/docs/cli/):

```bash
cargo insta test --test-runner nextest -p bacnet-cli   # writes .snap.new files
cargo insta review                                      # accept or reject each
```

The release-build render budget runs with:

```bash
cargo nextest run -p bacnet-cli --release -E 'test(drawing_a_full_device_table)' --no-capture
```

## 9. Platform support

| Capability | Linux | macOS | Windows |
|---|---|---|---|
| TUI (crossterm) | yes | yes (Terminal.app, iTerm2) | Windows Terminal; conhost on Windows 10 1703 and later. mintty and Git Bash are not supported terminals |
| B/IP | yes | yes | yes; wildcard local-address discovery is missing (#952) |
| BACnet/SC | yes | yes | yes |
| MS/TP participant (planned) | yes: USB auto-direction, kernel RS-485 (`TIOCSRS485`), `serial-gpio` | experimental: USB auto-direction only, `/dev/cu.*` | experimental: USB auto-direction only, `COMn` |
| MS/TP passive monitor (planned) | yes | experimental | experimental (host timer resolution limits timing accuracy) |
| Own-traffic capture (planned) | yes | yes | yes |
| Wire capture (libpcap) | yes (root or CAP_NET_RAW) | builds | needs the Npcap SDK |

- The `serial` feature is not Linux-only: tokio-serial and serialport build on
  all three OSes. Only kernel RS-485, `serial-gpio` and `ethernet` are
  Linux-only.
- MS/TP hardware evidence exists only for Linux USB adapters
  ([MS/TP qualification](../mstp-qualification.md)). USB-serial latency (the
  FTDI latency timer) can eat into the Clause 9.5 timing budget.
- Port names: Linux needs the `dialout` group; on macOS use `/dev/cu.*`, not
  `tty.*`.

## 10. Stack gaps (separate issues the TUI depends on)

| Issue | Gap | Needed by |
|---|---|---|
| #956 | Done: `BACnetClient::transport()` and the `AsBip` BBMD helpers. Was: no access to transport handles from a built `BACnetClient`: the SC state watch, SC drop counts and BIP counters are unreachable after build, and the BBMD helpers exist only on `BACnetClient<BipTransport>`, so `AnyTransport` loses them | #967, #957, #959 |
| #957 | No link-level frame tap on any transport, so BVLC control traffic, SC control messages and MS/TP Token, PFM and Reply-Postponed frames are invisible | #971 |
| #958 | No MS/TP passive monitor, per-station statistics or master list; `MstpDiagnostics` not exposed from builders | #970, #971 |
| #959 | SC reconnect, failover and disconnect causes appear only as tracing lines; no active-hub indication, heartbeat statistics or peer certificate; no failover or reconnect options on `ScClientBuilder` | #967 |
| #960 | I-Have is dropped; the client can't send Who-Is-Router-To-Network or What-Is-Network-Number or receive I-Am-Router-To-Network (Clause 6.4); the CLI `find` and `whois-router` are stubs | W1 (Who-Has, network map) |
| #938 | Foreign-device mode on `BipClientBuilder`; surfacing a refused registration (BVLC-Result NAK); a separate re-registration interval; `discover --bbmd` should send Distribute-Broadcast-To-Network (Annex J.5) | #966 |

Later gaps, not filed yet: routed variants for ReadRange, file access, DCC,
reinitialize, time sync, GetEventInformation, AcknowledgeAlarm and create and
delete object; GetEventInformation decoded in the client; client wrappers for
GetAlarmSummary, GetEnrollmentSummary, PrivateTransfer and TextMessage; client
TSM counters; an opt-in SC hub admin view of connected nodes.

## 11. Risks

| Risk | Mitigation |
|---|---|
| crossterm has not released since 0.29.0 and has breaking changes queued | Import through `ratatui::crossterm`; prefer widgets without a direct crossterm dependency |
| ratatui raises its MSRV in patch releases | Committed lock file, `--locked`, `resolver = "3"`, the MSRV CI job |
| Windows terminal quirks (duplicate key events, conhost resize, glyphs, mintty) | Press-only key filter; per-frame size; ASCII and box-drawing glyphs only; native Windows CI |
| TUIs are inaccessible to screen readers | One-shot parity for every action; text badges; `NO_COLOR`; low frame rate option |
| Main-thread stack overflow on Windows (#953) | Small draw path; boxed futures and state; worker on runtime threads; no server or hub hosting; 1 MiB stack guard in CI |
| Unbounded channels hide overload | Bounded channels; visible drop counter; per-wake drain budget; resync after loss |
| MS/TP timing on USB adapters, macOS and Windows | DedicatedThread; passive monitor by default; label macOS and Windows experimental until hardware evidence exists |
| An active MS/TP join or careless writes disturb a live site | Read-only default, write sessions, tiers, ledger, exit guard, safe-join wizard |
| Breaking the one-shot contract | The TUI is additive; contract tests stay unchanged; the shell is retired only by decision (#974) |
| Scope creep into a second product | Milestones ship independently; P1 and P2 items need their own issues |
| Audit logs leak secrets | Audit records redact passwords and never contain key material; SC key-log export is out of scope |
| Compile time and binary size growth | Non-generic views; the `tui` feature can be turned off; release-profile tuning measured in #973 |

## 12. Milestones

- **M0 foundation:** #961 (scaffold and devices screen, done), #962 (profiles
  and connection setup).
- **M1 BACnet/IP:** #963 (browser), #964 (safety and writes), #965 (watch
  list), #966 (BBMD and foreign device).
- **M2 BACnet/SC:** #967 (status and doctor), #968 (certificate lint).
- **M3 MS/TP:** #969 (MS/TP transport in the CLI), #970 (monitor and safe join).
- **M4 live capture:** #971.
- **Stack gaps:** #956 to #960, plus #938.

## 13. Decisions (maintainer, 2026-10-01)

- **Entry point:** ship `bacnet tui` first; a bare `bacnet` switches to the TUI
  after M1 and the shell is retired later (#974).
- **Packaging:** one binary with a default-on `tui` feature; the code lives in
  `tui/` inside bacnet-cli with a shared `core/`.
- **Terminal size:** 80x24 functional, 120x40 full layout, a notice below the
  minimum.
- **MSRV and resolution:** `resolver = "3"` with MSRV 1.93; the MSRV CI job is
  the backstop.
- **Profiles:** TOML in the platform config directory with an environment
  override; flags-only runs stay supported.
- **Audit log:** on by default for mutations and safety events.
- **Certificates:** x509-parser behind `sc-tls`; where our hostname and IP check
  is stricter than the Annex AB.7.4 default, explain it rather than relax it.
- **MS/TP release status:** in the Linux CLI release for 0.12.0; experimental in
  macOS and Windows CI until there is hardware evidence for at least one USB
  adapter per OS.
- **One-shot write guard:** a separate issue (#972); the one-shot contract is
  untouched by this epic.
- **Lab PKI generator:** out of scope for v1.
- **Release profile size tuning:** measured separately (#973) before deciding.
- **Timing:** the TUI targets 0.14.0, so it holds up neither 0.12.0 nor
  0.13.0.

## 14. References

- Ratatui 0.30 highlights <https://ratatui.rs/highlights/v030/>, backends
  <https://ratatui.rs/concepts/backends/comparison/>, the Elm architecture
  <https://ratatui.rs/concepts/application-patterns/the-elm-architecture/>,
  snapshot testing <https://ratatui.rs/recipes/testing/snapshots/>, panic hooks
  <https://ratatui.rs/recipes/apps/panic-hooks/>
- crossterm <https://github.com/crossterm-rs/crossterm>
- insta <https://insta.rs/>
- Spec clauses (paraphrased above): 6.4 network-layer messages; 9.5 MS/TP; 16.1
  DCC; 16.9 Who-Has; 16.10 Who-Is; 19.2 command prioritization; 19.6 audit
  reporting; Annex J (J.5) B/IP and BBMD; Annex AB (AB.1.5, AB.7.4) BACnet/SC
- Repo: `crates/bacnet-cli/`, `crates/bacnet-client/src/client/`,
  `crates/bacnet-transport/src/`, [CLI reference](../CLI.md),
  [MS/TP qualification](../mstp-qualification.md),
  [mutation policy](../mutation-policy.md), [CI](../ci.md)
