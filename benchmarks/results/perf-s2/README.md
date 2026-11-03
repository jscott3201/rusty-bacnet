# #1979(b): out-of-resources Aborts at 100 clients, by count (S2)

Part of [#1979](https://github.com/jscott3201/rusty-bacnet/issues/1979),
slice S2 of the counts-first plan. Section 10.1 of Benchmarks.md found
that every failure in the 100-client B/IP stress step was an Abort with
reason OUT_OF_RESOURCES, at 0.07% to 0.47% of requests depending on the
revision, and left the cause open. This directory shows it is the server's
request admission policy at work, with counts of refusals per fixed amount
of work instead of rates inside a time window.

## The policy

`RequestAdmissionPolicy::default()` allows 64 confirmed handlers at once and
keeps 4 of them for DeviceCommunicationControl recovery, so 60 ordinary
confirmed requests can run at a time, at most 16 from one peer. A request
that finds the 60 slots taken is refused at once: dispatch answers it with
an Abort (OUT_OF_RESOURCES) from one of 8 Abort workers, or drops it
silently, and counts it, when all 8 are busy. A handler keeps its slot
until its reply has been handed to the transport.

## The deterministic case (E3)

`crates/bacnet-server/src/server/default_admission_tests.rs` dispatches 61
ReadProperty requests from 61 different stations under the default policy
before any handler runs, with the test transport holding every reply in its
send. Exactly one Abort (OUT_OF_RESOURCES, the 61st request's invoke ID)
comes back, the other 60 are answered, and the new peak counter reads 60.
Raising the reserve to 3 (61 ordinary slots) fails the test.

## Counts at a fixed request count (E1, E2)

`stress-test clients --transport bip --requests 2000` gives each client
exactly 2,000 ReadProperty requests, one outstanding at a time;
`--max-confirmed-in-flight` sets the server's limit, and each step reports
the most confirmed handlers that were live at once (`confirmed_peak`,
the peak of the server's `confirmed_active` counter, which would include
recovery handlers; these clients send none). Three repetitions each,
release build, macOS 27.2 on an Apple M5 laptop shared with other work,
B/IP on loopback. Counts of refusals
don't depend on the machine's speed the way rates in a window do, but which
requests overlap still depends on scheduling, so the counts vary between
runs; macOS is fine for counting them. `runs.tsv` has every step.

**E1: 100 clients, 200,000 requests per run.**

| Limit | Aborts per 100,000 requests (three runs) | Peak live handlers | Silent drops |
| --- | --- | --- | --- |
| 64 (default; 60 ordinary) | 410.5, 597.0, 301.5 | 60, 60, 60 | 31, 33, 31 |
| 104 (100 ordinary) | 0, 5.0, 0 | 94, 100, 99 | 0, 0, 0 |

**E2: the default policy against the number of clients.**

| Clients | Aborts per 100,000 requests (three runs) | Peak live handlers |
| --- | --- | --- |
| 50 | 0, 0, 0 | 49, 52, 50 |
| 60 | 0, 5.8, 0 | 60, 60, 60 |
| 61 | 0, 106.6, 12.3 | 59, 60, 60 |
| 64 | 10.2, 553.1, 36.7 | 60, 60, 60 |
| 80 | 307.5, 397.5, 228.8 | 60, 60, 60 |
| 100 | 258.0, 191.0, 438.0 | 60, 60, 60 |

Every failure was an Abort with reason OUT_OF_RESOURCES; every refusal was
global (no per-peer refusals); in every step, Aborts plus silent drops equal
the refusals the server counted.

## What held and what didn't

- **E3 held** exactly: 60 answered and one Abort.
- **E1 held in two of three runs.** With 100 ordinary slots for 100 clients,
  one run still refused 10 requests, with the peak at 100.
- **E2 held but for the edge.** No Aborts at 50 clients, and Aborts in every
  run from 64 clients up. But 60 clients drew 7 Aborts in one run, and the
  peak reached 52 with 50 clients.

Both misses have one cause: the slot outlives the reply. A client can
receive its reply and send its next request before the server task that
answered it has finished and released its slot, so for a moment one client
holds two slots. Live handlers can then exceed the number of clients, as
the peak of 52 with 50 clients shows, and a limit equal to the client count
still refuses now and then. Above the limit the rate swings between runs
(191 to 597 per 100,000 at 100 clients) because it depends on how many
requests arrive inside those moments, which is scheduling, not per-request
cost; that also fits the spread of rates between revisions in section 10.1.

**Silent drops come from the start of a run.** From 80 clients up, each run
silently dropped 12 to 34 requests when all 8 Abort workers were busy. Each
such client waits out its 5-second request timeout and retries, and the
retry succeeds: none of the 24 steps had a failure other than the Aborts.
Those runs took 5.5 to 7 seconds against 1.5 to 3.6 without drops; the
difference is that timeout, not lower throughput. The independent review
reran the release binary with fewer requests per client and found the drops
barely depend on run length: at 80 clients, 3 drops with 5 requests per
client, 7 with 20, 12 with 200 and 18 with 2,000; at 100 clients, 25 with 20
and 30 with 200. With one request per client there were no drops at 80
clients and no Aborts at 61 to 80. So the drops come early, while the
clients still send in step and more than 8 refusals arrive together. This is
documented behaviour of the policy, not a fault; more Abort workers would be
a separate decision.

## Reproduce

```bash
cargo build --release -p bacnet-benchmarks --bin stress-test
ST=target/release/stress-test
$ST clients --transport bip --steps 100 --requests 2000
$ST clients --transport bip --steps 100 --requests 2000 --max-confirmed-in-flight 104
$ST clients --transport bip --steps 50,60,61,64,80,100 --requests 2000
cargo nextest run -p bacnet-server -E 'test(default_policy_answers_60)'
```

All nine runs took about 90 seconds.
