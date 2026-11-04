# Allocations (S5): the ReadProperty value and the B/IP send buffer

Part of [#1978](https://github.com/jscott3201/rusty-bacnet/issues/1978):
item 3 (the ReadProperty-ACK built in place), item 5 (the B/IP buffer per
send) and item 4 (the second box for services other than ReadProperty),
each counted against its own baseline with the counting harness
(Benchmarks.md section 10.4). Section 10.9 there summarizes them. Items 3
and 5 are accepted; item 4 is not.

All instruction figures are callgrind instruction reads per request,
without the harness's counting allocator. "Calls" are allocator calls per
request and "bytes" the bytes requested from the allocator per request.
`ct` is Tokio's current-thread runtime, `mt` its multi-thread runtime with
two workers. The floor is the range of the twelve A and A' values (two
clean builds of the baseline) from the same matrix; A and A' were
byte-identical in every run. Each run built like the release with the
fixed allocator settings, in local Docker on an Apple M5 laptop, image
`rb-perf-counts:ac28c4cf5d0a` (libc6 2.39-0ubuntu8.9, Valgrind 3.22.0,
Rust 1.99.0), in the order A B A' A' B A three times.

**Baselines.** The two items were written on dev `c252ccafc` and merged
with dev `32c21b87d` (after S4, #2374) before these counts, so the counts
are of the code the pull request merges:
- item 3 is dev `32c21b87d` against `3b243714c`, which is that dev with
  item 3's commit (`d20713a80`) cherry-picked (a local commit, not pushed);
- item 5 is `3b243714c` against `652f94c09`, the merge of item 5
  (`6620adebc`) and dev. Its tree is `3b243714c` plus item 5 and nothing
  else.

The branch head after these counts adds this directory, a merge with dev
`059fe5b99` (#2376's check of legacy Event_Parameters values, which the
in-place ACK now runs on the encoded value) and the review's fixes.

## Item 3: the ReadProperty value encoded in place

**Where the cost was.** The ReadProperty-ACK encoded the value read into a
buffer of its own, copied that buffer into a `Vec` for the ACK, and the
ACK encoder copied it a second time into the service ACK: two allocations
and two copies per request.

**The change** (`bacnet-device-core/src/read/property.rs`, `encode_ack`).
The ACK encoder writes the members around an empty value into an
18-octet stack buffer: the object and property identifiers, the optional
array index and the opening tag, then the one-octet closing tag. The value
is encoded straight into the service ACK between the two. The output is
the encoder's, byte for byte; `property_ack_tests.rs` compares the two for
values of every shape and size, identifiers at their limits and each array
index form.

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Bytes A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| ReadProperty | ct | 32,134.1 | 31,672.4 | -461.7 | 13.5 | 6 of 6 | 22.0002 / 20.0002 | 2,196.02 / 2,183.02 | PASS |
| ReadProperty | mt | 33,442.7 | 32,967.0 | -475.7 | 103.6 | 6 of 6 | 22.0002 / 20.0002 | 2,196.02 / 2,183.02 | PASS |
| WriteProperty | ct | 40,305.3 | 40,306.8 | +1.5 | 10.0 | 3 of 6 | 27.0002 / 27.0002 | 7,995.02 / 7,995.02 | PASS |
| WriteProperty | mt | 41,554.3 | 41,566.9 | +12.6 | 67.9 | 3 of 6 | 27.0002 / 27.0002 | 7,995.02 / 7,995.02 | PASS |
| ReadPropertyMultiple | ct | 91,431.5 | 91,431.1 | -0.5 | 20.6 | 3 of 6 | 141.0005 / 141.0005 | 18,781.05 / 18,781.05 | PASS |
| ReadPropertyMultiple | mt | 93,500.9 | 93,355.2 | -145.7 | 918.0 | 4 of 6 | 141.0005 / 141.0006 | 18,781.05 / 18,781.06 | PASS |

**Verdict: accepted.** ReadProperty falls by about 470 instructions per
request: 34.2 times the floor on the current-thread runtime and 4.6 times
on the multi-thread one, with all pairs lower. It makes two fewer
allocator calls per request (22 to 20) and requests 13 fewer bytes. In
`item3/profiles/rp-ct.md`, `malloc`, `free` and `_int_free` account for
312 of the instructions saved. WriteProperty and ReadPropertyMultiple
don't read through this path, and move inside their floors.

The server's copy of the whole APDU on the confirmed-reply path
(`requests/confirmed_response.rs`, the plan's other site for this item)
is unchanged: removing it changes a file the #2303 work also changes.

## Item 5: the B/IP send buffer

**Where the cost was.** Every B/IP unicast allocated a buffer, encoded the
four-octet BVLL header into it, copied the NPDU in behind the header and
sent that: an allocation and a copy per send, two per round trip.

**The change** (`bacnet-transport/src/bip/datagram.rs`, `send.rs` and
`socket.rs`). An `OutgoingDatagram` holds the BVLL header in ten octets of
its own and borrows the NPDU, and the socket sends the two parts as one
datagram with a vectored send (socket2's `send_to_vectored`, inside
Tokio's `try_io`); the would-block path waits for the socket and sends the
same way. `OutgoingDatagram::split` takes the header from the BVLL codec
itself, through a sink that keeps the header and checks that the rest is
the caller's NPDU in one write, so the wire bytes are still the encoder's.
The tap joins the two parts only when it has a subscriber. The transport
API, the network layer and `bacnet-npdu` are unchanged; the other
transports and the B/IP broadcast and BBMD paths still encode whole
frames.

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Bytes A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| ReadProperty | ct | 31,668.7 | 31,250.1 | -418.6 | 9.4 | 6 of 6 | 20.0001 / 18.0001 | 2,183.02 / 2,127.02 | PASS |
| ReadProperty | mt | 32,968.2 | 32,543.6 | -424.6 | 25.4 | 6 of 6 | 20.0002 / 18.0002 | 2,183.02 / 2,127.02 | PASS |
| WriteProperty | ct | 40,309.4 | 39,887.9 | -421.5 | 101.5 | 6 of 6 | 27.0002 / 25.0002 | 7,995.02 / 7,944.02 | PASS |
| WriteProperty | mt | 41,562.3 | 41,083.7 | -478.6 | 109.9 | 6 of 6 | 27.0002 / 25.0003 | 7,995.02 / 7,944.03 | PASS |
| ReadPropertyMultiple | ct | 91,430.4 | 90,986.4 | -444.0 | 30.2 | 6 of 6 | 141.0004 / 139.0004 | 18,781.04 / 18,496.04 | PASS |
| ReadPropertyMultiple | mt | 93,582.7 | 92,971.4 | -611.3 | 530.0 | 5 of 6 | 141.0005 / 139.0005 | 18,781.05 / 18,496.05 | PASS |

**Verdict: accepted.** ReadProperty falls by about 420 instructions per
request: 44.3 times the floor on the current-thread runtime and 16.7 times
on the multi-thread one, with all pairs lower. WriteProperty falls by
421.5 and 478.6 (4.2 and 4.4 times its floors), with all pairs lower, and
ReadPropertyMultiple by 444.0 on the current-thread runtime. Its
multi-thread figure, 611.3, is only 1.2 times its floor of 530.0, with 5
of 6 pairs lower, so it is no measured saving. Every round trip makes two fewer
allocator calls (one per send, request and reply); the bytes requested
fall by the two frames' size. In `item5/profiles/rp-ct.md`:
- `encode_bvll` goes from the unicast path (192 instructions per request);
- `malloc`, `free` and `_int_free` fall by 306;
- `SendTo::poll` falls by 104, and the unicast send, which now splits the
  header itself, rises by 132.

Callgrind counts only user-space instructions, so a vectored send's cost
in the kernel against a plain one is not in these figures. The wall-clock
check after S5 (the plan's S6) covers it.

`check-no-std.sh` and `check-firmware-link.sh` pass on `9b9fa96b9`.
The change is in the Tokio transport, which no firmware image links; the
B/IP node, BBMD and router images, in bytes used against the budget:

| Image | Target | Flash | RAM |
| --- | --- | ---: | ---: |
| node | thumbv7m | 16,636 of 18,000 | 240 of 272 |
| node | thumbv6m | 17,608 of 19,088 | 240 of 272 |
| node | riscv32imc | 18,838 of 20,096 | 240 of 272 |
| node | thumbv7em | 16,580 of 17,936 | 240 of 272 |
| bbmd | thumbv7m | 12,072 of 12,832 | 240 of 272 |
| bbmd | thumbv6m | 13,312 of 13,872 | 240 of 272 |
| bbmd | riscv32imc | 14,266 of 15,072 | 240 of 272 |
| bbmd | thumbv7em | 12,020 of 12,752 | 240 of 272 |
| router | thumbv7m | 43,272 of 47,024 | 15,632 of 17,376 |
| router | thumbv6m | 45,836 of 49,056 | 15,632 of 17,376 |
| router | riscv32imc | 49,266 of 52,960 | 15,632 of 17,376 |
| router | thumbv7em | 43,168 of 46,848 | 15,632 of 17,376 |

## Item 4: one box per request handler (not accepted)

**What it was.** Admission boxes the future a confirmed request's task
runs. For any service but ReadProperty, that future was an `Either`
holding a second box, the general handler's, so the handler took two
allocations besides the task. The candidate (`68f5f1411`, on top of item
5's first form `e9adf9630`) spawned a ReadProperty and every other
service as two separate task types, the general handler unboxed. Neither
commit is on the branch.

**Counts** (on the old base: `e9adf9630` against `68f5f1411`, aimed at
WriteProperty, in `item4/`):

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Bytes A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| ReadProperty | ct | 32,364.2 | 32,308.2 | -56.0 | 9.3 | 6 of 6 | 18.0002 / 18.0002 | 2,127.02 / 2,119.02 | PASS |
| ReadProperty | mt | 33,682.7 | 33,616.9 | -65.8 | 158.5 | 6 of 6 | 18.0005 / 18.0005 | 2,127.05 / 2,119.06 | PASS |
| WriteProperty | ct | 41,033.6 | 40,764.6 | -269.0 | 10.2 | 6 of 6 | 25.0003 / 24.0003 | 7,960.03 / 7,024.03 | PASS |
| WriteProperty | mt | 42,285.0 | 41,977.4 | -307.6 | 187.4 | 5 of 6 | 25.0010 / 24.0005 | 7,960.10 / 7,024.05 | FAIL |
| ReadPropertyMultiple | ct | 91,501.1 | 91,195.8 | -305.4 | 50.6 | 6 of 6 | 139.0004 / 138.0004 | 18,512.04 / 17,576.04 | PASS |
| ReadPropertyMultiple | mt | 94,247.1 | 93,963.8 | -283.4 | 824.5 | 5 of 6 | 139.0004 / 138.0004 | 18,512.04 / 17,576.04 | PASS |

**Verdict: not accepted.** WriteProperty on the multi-thread runtime fell
by 307.6 instructions per request, 1.6 times its floor of 187.4; the rule
asks for more than 3 times, with every pair lower, and one pair of six was
not. The change is dropped from the branch and was not recounted.

**What the profile showed** (`item4/attribution-wp-mt.md`, and
`item4/profiles/wp-ct.md` for the same on the current-thread runtime):
- one allocator call fewer per WriteProperty and ReadPropertyMultiple
  request (25 to 24, 139 to 138), and 936 fewer bytes requested: the
  second box;
- `memcpy` falls by 126 instructions per request (the handler future
  moved into that box), and `malloc`, `free` and `_int_free` by about 145;
- the task's spawn, poll and drop code only moves to new names at about
  the same size (the two spawn functions 375 instructions per request
  before, 350 after), so the saving is the one allocation and its copy.

ReadProperty did not rise this time (it fell by 56 and 66), unlike the
+280 the #1821 experiment saw. The saving
is real but about 300 instructions, below what the multi-thread runtime's
WriteProperty spread can confirm.

## Both items together: the official count

The manual Perf counts workflow compared dev `32c21b87d` with `9b9fa96b9`
(this directory's commit) on GitHub's arm64 runner (run 41890882196,
`official/`). Its candidate binary is byte-identical to the local item 5
candidate's (`e3c55dbd2c77f605`).

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Bytes A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| ReadProperty | ct | 32,145.0 | 31,261.5 | -883.5 | 12.0 | 6 of 6 | 22.0005 / 18.0005 | 2,196.05 / 2,127.05 | PASS |
| ReadProperty | mt | 33,539.8 | 32,632.7 | -907.2 | 46.6 | 6 of 6 | 22.0008 / 18.0007 | 2,196.08 / 2,127.07 | PASS |
| WriteProperty | ct | 40,323.2 | 39,902.4 | -420.8 | 111.5 | 6 of 6 | 27.0006 / 25.0006 | 7,995.06 / 7,944.06 | PASS |
| WriteProperty | mt | 41,581.6 | 41,128.8 | -452.8 | 146.3 | 6 of 6 | 27.0009 / 25.0009 | 7,995.09 / 7,944.09 | PASS |
| ReadPropertyMultiple | ct | 91,289.5 | 91,015.2 | -274.2 | 873.0 | 4 of 6 | 141.0011 / 139.0011 | 18,781.11 / 18,496.11 | PASS |
| ReadPropertyMultiple | mt | 94,540.0 | 94,040.8 | -499.2 | 350.6 | 6 of 6 | 141.0014 / 139.0014 | 18,781.15 / 18,496.15 | PASS |

ReadProperty falls by 883.5 and 907.2 instructions per request (73.4 and
19.5 times its floors), close to the two local comparisons added up
(880.3 and 900.3), with four allocator calls fewer. The later merge with
#2376 adds a property comparison and two length reads per ReadProperty
and was not counted.

## Files

- `summary.json`: the four comparisons' rows (medians, deltas, floors,
  pairs, allocation calls and bytes, verdicts) and commits.
- `official/`: the workflow run's `summary.md` and `summary.json`.
- `item3/`, `item5/`, `item4/`: each comparison's `summary.md` and
  `summary.json` from `perf_counts.py`, the #1978 item shares
  (`items.md`) and per-function and per-line slopes (`profiles/`).
  `item4/attribution-wp-mt.md` is the `attribute` output for the failing
  row.

Reproduce (item 3's candidate is dev `32c21b87d` with `d20713a80`
cherry-picked):

```bash
git switch -c item3-on-dev 32c21b87d && git cherry-pick d20713a80
python3 scripts/perf/perf_counts.py run --baseline 32c21b87d --candidate item3-on-dev \
  --workloads rp,wp,rpm --out /tmp/item3
python3 scripts/perf/perf_counts.py run --baseline item3-on-dev --candidate 652f94c09 \
  --workloads rp,wp,rpm --out /tmp/item5
python3 scripts/perf/perf_counts.py run --baseline e9adf9630 --candidate 68f5f1411 \
  --workloads rp,wp,rpm --aim wp --out /tmp/item4
```
