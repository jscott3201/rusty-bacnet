# Hub broadcast relay budget (#518 first slice)

This is an always-on, tunable **local overload policy**, not a new BACnet wire
requirement or a claim of full Annex AB conformance.

## Admission and exclusions

Only an already-classified `HubRelayTarget::Broadcast` spends tokens. The three
eligible forwarding families (Encapsulated-NPDU, Unknown, Proprietary) share one
sender bucket and one hub bucket. Checks occur after existing admission and
activity accounting, but before relay encoding, recipient collection, copying,
or fanout. Sender identity is the handler's registered VMAC lease, never a
peer-supplied origin; the bucket lives in that handler, without a per-VMAC map.

Unicast is unchanged, including relayed unicast whose destination is removed.
Address-Resolution/ACK, Advertisement/Solicitation and Results are unicast
transit or existing locally rejected/dropped operations, not shared-state
control processing. Their processing and local rejection/write budgets are
explicitly out of scope. No NPDU decoding, heartbeat/activity/deadline,
replacement, retirement, recipient size gating, or send timeout policy changes.

Exhaustion silently drops the **whole broadcast request**; it does not send a
NAK, close the sender, or queue delayed work. Source check: local licensed
Standard 135-2020, Annex AB.2, printed p. 1383 (PDF position 1385), and AB.2.4,
printed pp. 1386–1387 (PDF positions 1388–1389). Broadcasts must not elicit responses, so
BVLC-Result is not a suitable rate-drop response. This note paraphrases the
source and does not claim the numeric quotas come from it.

## Defaults and derivation

| Bucket | Continuous refill | Initial/maximum burst |
|---|---:|---:|
| Registered sender connection | 128 broadcasts/s | 1,024 broadcasts |
| Hub aggregate | 512 broadcasts/s | 4,096 broadcasts |

In-repository scale evidence:

- `sc/advertisement.rs`: solicited advertisements have a 1-second local spacing.
- `sc/heartbeat.rs`: default heartbeat interval 30 seconds, timeout 60 seconds;
  `sc_hub/connection.rs` sweeps every 30 seconds. Neither is gated here.
- `bacnet-client/src/tsm.rs`: default APDU/segment retransmission timeouts are
  6,000 ms, with three retries. These are context, not broadcast retry promises.
- `sc_hub/proprietary_transit_tests.rs`: the broadcast fanout case sends two
  back-to-back messages; its opaque unicast matrix sends 32 cases. These small
  functional bursts are evidence of scale, not a deployment throughput study.
- `sc_hub/unknown_transit_tests.rs`: its all-functions/opaque-options test sends
  243 + (3 * 2 * 4 * 2) = 291 broadcasts from one sender without explicit pacing.
- `sc_hub/retirement_capacity_tests.rs`: the terminal-relay capacity test sends
  513 broadcasts from one source among 2,052 target sessions, also without
  pacing. Correct behavior must not depend on TLS/test execution being slow.

Use the existing one-second scale, but allow 128 sender broadcasts in that
second and eight seconds of burst credit. A 1,024-message burst is the next
power of two above the existing 513-broadcast test, with substantial headroom.
The initial 256-message candidate failed those existing high-volume tests and
was raised rather than changing their behavior. This leaves room relative to
the timer scales: 768 sustained broadcasts per 6-second retry interval, or
3,840 per default heartbeat interval, plus the initial burst. The global budget
permits four sender-rate streams, but cannot grow with the number of senders.
The factors 128, eight-second burst, and four sender streams are deliberately
generous local choices, **not measured safe limits for every deployment**.

For any elapsed interval `t` seconds, admissions are at most `burst + rate*t`
(rounded down to whole tokens), not a fixed-window reset quota. The existing
256-client cap means aggregate fanout attempts are at most
`255 * (4096 + 512*t)` by default. Tokens count broadcast requests, not bytes
or actual recipients; even no-recipient/oversized-recipient requests spend a
token. Bucket credit is capped, so idle time cannot accumulate unbounded work.
This bounds admitted fanout, not ingress parsing, OS buffers, or fair service
under an aggregate flood. Once the global bucket is empty, any sender can drop.

## Configuration, lifetime, and diagnostics

Use `ScHubTlsConfig::with_broadcast_rate_policy(ScHubBroadcastRatePolicy { .. })`
before any existing public startup API. Every startup validates all four bounds
before binding: zero and values above `u64::MAX / 1_000_000_000` are rejected.
No disable sentinel exists. Raise bursts for measured legitimate discovery
peaks; lower aggregate rates for large/slow fanouts; review recipient count and
observed drop counters before raising sustained rates.

There is O(1) state per handler and O(1) state per hub. A new connection starts
with a fresh sender bucket; replacement/reconnection cannot reset the aggregate.
Configuration clones start independent hubs. The existing task owner holds the
aggregate and explicitly scopes each accepted connection to it; no background
refill task, timer, queue, or lifecycle control is added. Refill uses monotonic
Tokio time and integer nanotokens, retaining partial tokens exactly. The global
mutex protects only fixed arithmetic, with no I/O or await while locked.

The sender check runs first. Sender-rejected requests do not debit the global
bucket. A request passing the sender check spends that token even if rejected
globally. `ScHub::broadcast_drop_counts()` exposes two saturating lifetime `u64`
counters (`sender_exhausted`, `global_exhausted`); each rate drop counts exactly
once. They are diagnostics only, never admission inputs. Individual reads are
atomic; the pair is not transactional under active traffic. Counts remain
readable after stop. No unbounded per-sender metric labels or maps are retained.
Warnings follow the existing `malformed_diag` one-per-second per-handler idiom
with suppressed-event summaries. A separate throttle keeps malformed-frame
diagnostics independent of rate-drop diagnostics.

## Evidence

`broadcast_rate_tests.rs` covers exact bursts/sub-token boundaries, fractional
refill under repeated rejection, long-idle clamping, stale clock samples,
zero/overflow/max bounds, exact and saturating counters, sender isolation,
unicast bypass, and a 16-thread aggregate admission race.

`broadcast_rate_live_tests.rs` exercises public TLS hub startup with frozen
Tokio time and ordered wire barriers: mixed-family single-sender flooding,
other sender/registration/heartbeat progress, exact refill boundary, concurrent
multi-sender aggregate flooding and replacement, high-rate byte-exact unicast
and control traffic, all four startup APIs rejecting bad bounds, and independent
cloned-config hubs. Existing hub suites are left unmodified.
