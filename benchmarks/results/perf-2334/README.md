# Three codec savings for builds without link-time optimization (#2334)

[#2334](https://github.com/jscott3201/rusty-bacnet/issues/2334), part of
[#1979](https://github.com/jscott3201/rusty-bacnet/issues/1979): the three
codec changes that slice S1 ([perf-s1](../perf-s1/README.md)) found and left
open, each made in a commit of its own and counted on its own with the
counting harness (Benchmarks.md section 10.4). Section 10.12 there
summarizes it.

All instruction figures are callgrind instruction reads per iteration of a
codec workload, without the harness's counting allocator. "Calls" are
allocator calls per iteration. Every matrix built the baseline twice (A and
A') and the candidate once (B) and ran them in the order A B A' A' B A
three times. In every codec matrix the two baseline builds were
byte-identical and their twelve values equal, so the noise floor is zero
and any change counts. Everything ran in local Docker on an Apple M5 laptop, image
`rb-perf-counts:ac28c4cf5d0a` (libc6 2.39-0ubuntu8.9, Valgrind 3.22.0,
Rust 1.99.0), with the harness's fixed allocator settings.

**Two build profiles.** "Release-like" is the harness as it is: whole-program
link-time optimization and one codegen unit, as the published binaries
build. "Default profile" is the harness with `lto` and `codegen-units`
deleted from its `[profile.release]` and nothing else changed, as in
perf-s1: what a downstream crate gets from `cargo build --release` unless
it asks for more.

**The rule.** An item is kept when its workloads fall in at least one
profile, nothing rises in the other, allocator calls don't rise, and the
round trips are not higher. One that rises in either profile is dropped.

| Item | Commits (A to B) | Release-like | Default profile | Verdict |
| --- | --- | --- | --- | --- |
| 1. Direct `decode_npdu` | `9e6bb0981` (dev) to `3b042d0e7` | NPDU decode 172 to 153 (-19) | 219 to 154 (-65) | **Kept** |
| 2. The `BytesMut` sink's `write` inlined | `3b042d0e7` to `a13238fbb` | ReadProperty request encode 306 to 306 (0) | 705 to 611 (-94) | **Kept** |
| 3. `encode_npdu` without the zero-fill | `a13238fbb` to `23906182f` | NPDU encode 384 to 359 (-25) | 446 to 447 (+1); whole request +10 | **Dropped** (reverted in `493b8647b`) |

## 1. Direct `decode_npdu`: kept

Since #1788, `decode_npdu` parsed every frame through
`bacnet_npdu::Npdu::decode`, the borrowed view, and then copied the view's
fields into the owned `Npdu`. It now reads a frame the view accepts
straight into the owned `Npdu` (`accepted_header`, in
`crates/bacnet-encoding/src/npdu.rs`), and hands any other frame to the
view, which names the fault (`decode_through_view`, the whole of the old
path). So an accepted frame is parsed once, into its final form, and a
refused frame gets the error it got before, and the same one of several.

A is dev `9e6bb0981`, B is `3b042d0e7`.

| Workload | Release-like A | B | B - A | Default profile A | B | B - A | Calls A / B |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Decode NPDU, 100-octet payload | 172 | 153 | **-19** | 219 | 154 | **-65** | 0 / 0 |
| Encode NPDU, 100-octet payload | 384 | 384 | 0 | 446 | 446 | 0 | 1 / 1 |
| Encode ReadProperty request | 306 | 306 | 0 | 705 | 705 | 0 | 1 / 1 |
| Decode ReadProperty request | 180 | 180 | 0 | 169 | 169 | 0 | 0 / 0 |
| Encode confirmed-request APDU | 312 | 312 | 0 | 522 | 522 | 0 | 1 / 1 |
| Decode confirmed-request APDU | 222 | 222 | 0 | 239 | 239 | 0 | 0 / 0 |
| Whole ReadProperty request (NPDU, APDU, service) | 1,412 | 1,412 | 0 | 2,157 | 2,157 | 0 | 5 / 5 |

In the default profile the call to `Npdu::decode` is gone (46 instructions,
one call per decode) and `decode_npdu` itself runs 19 fewer (127 to 108):
`item1-direct-decode/default-profile/attribution-decode_npdu_100b_payload.md`.
Release-like, the view's decoder was already inlined, and the 19 are what
going through the view and copying it into the owned form cost. The scratch patch of perf-s1 measured 154
and 158; this one returns from the direct path before the fallback is
reached, which a first shape of it didn't (159 and 164).

**The error contract.** `npdu/direct_decode_tests.rs` compares `decode_npdu`
with a decode written in the test on the view alone, as `decode_npdu` was
before, result for result, and checks that the direct path accepts exactly
the frames the view accepts:

- 72 valid frames of every shape (no destination, or one with an address
  of 0, 1, 6 or 18 octets; no source, or one of 1, 6 or 18 octets; an APDU,
  a standard message or a proprietary one), each with every header bit
  flipped, every value of the control octet, every value of each length
  octet over the same octets, each address at every length from 0 to 26
  with its octets present, and each network number set to 0 and to 65535;
  and every cut of each: 1,477,869 inputs. The test asserts that they drew
  every outcome the view has: accepted, each of its four malformations,
  and an over-long DADR or SADR with and without a source behind it or a
  DNET.
- The 397 seed files of the encoding fuzz crate, as they are and behind each
  of the 256 control octets, and every cut of each: 1,463,849 inputs. No
  fuzz target decodes an NPDU, so there were no NPDU seeds to replay.
- 2,048 generated frames with any value in any field, and 2,048 strings of
  up to 63 arbitrary octets, with every cut of each.

Both of these changes fail it, and were then undone: a direct path that
accepts an SNET of 65535, and a fallback that reports an SLEN of 0 as a
reserved network number.

## 2. The slice sink: one `#[inline]` kept, on the `BytesMut` sink

Since #1780 the ReadProperty request encodes through the sink encoders:
each field is staged in a small stack buffer, the request in another
(`staged`), and that is appended to the `BytesMut`. The sink encoders are
generic, so the crate that calls them compiles its own copy; the sinks'
`write` methods are not, and without link-time optimization each was a call
across the crate boundary.

Seven shapes were counted, each `3b042d0e7` with the change its row names.
Only the kept one has a full matrix; the others were counted two shapes to
a quick-mode run (one build of each, the order A B B A, floors from the
full matrices), for comparison only. The whole
ReadProperty request moved by the same amount as the request encode in
every row.

| Shape | Release-like | Change | Default profile | Change |
| --- | ---: | ---: | ---: | ---: |
| `3b042d0e7`, before | 306 | | 705 | |
| `#[inline]` on the slice sink's `write` | 328 | +22 | 608 | -97 |
| ... and on the `BytesMut` sink's `write` | 328 | +22 | 395 | -310 |
| ... and on the scalar encoders' `write_tagged` | 328 | +22 | 395 | -310 |
| **`#[inline]` on the `BytesMut` sink's `write` alone (kept)** | **306** | **0** | **611** | **-94** |
| The request encoded straight into the `BytesMut`, no staging | 309 | +3 | 480 | -225 |
| ... with `#[inline]` on the `BytesMut` sink's `write` | 386 | +80 | 515 | -190 |
| ... with `#[inline]` on both sinks' `write` | 308 | +2 | 330 | -375 |

- **Kept: `#[inline]` on `<BytesMut as EncodeSink>::write`**
  (`crates/bacnet-encoding/src/sink.rs`), `a13238fbb`. It is the only shape
  that leaves the release-like build where it was. In the default profile
  the request loses one call to that `write` (42 instructions), one call to
  `write_tagged::<&mut [u8]>` and one `memcpy`:
  `item2-bytesmut-sink-write/default-profile/attribution-encode_read_property_request.md`.
- **Not taken: the slice sink's `write`, the issue's first suggestion.**
  With it inlined, a release-like build runs 22 instructions more per
  request, not fewer: it then makes a `memset` call per request (12
  instructions) where it made none, and `ReadPropertyRequest::encode` runs
  10 more.
- **Not taken: encoding straight into the `BytesMut`, the issue's second
  suggestion.** Alone it costs a release-like build 3 instructions. With
  both `write`s inlined it is the cheapest default-profile shape by far
  (330, less than half of 705), and release-like it is 2 above where it
  started: it saves a `memcpy` call (16), and has the same 12-instruction
  `memset` call and 6 more in `ReadPropertyRequest::encode`.

So about 280 instructions per ReadProperty request are still there to
take in a build without link-time optimization (611 against 330). What
stands in the way is 2 instructions, or 22, in a build with it, and in
both cases a `memset` call that appears once the slice sink's `write` is
inlined. Which fill it is was not traced; the staging buffers are
zero-filled and then partly written over, and that is the likely one.

A is `3b042d0e7`, B is `a13238fbb`, full matrices:

| Workload | Release-like A | B | B - A | Default profile A | B | B - A | Calls A / B |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Encode ReadProperty request | 306 | 306 | 0 | 705 | 611 | **-94** | 1 / 1 |
| Whole ReadProperty request (NPDU, APDU, service) | 1,412 | 1,412 | 0 | 2,157 | 2,063 | **-94** | 5 / 5 |
| Decode NPDU, 100-octet payload | 153 | 153 | 0 | 154 | 154 | 0 | 0 / 0 |
| Encode NPDU, 100-octet payload | 384 | 384 | 0 | 446 | 446 | 0 | 1 / 1 |
| Decode ReadProperty request | 180 | 180 | 0 | 169 | 169 | 0 | 0 / 0 |
| Encode confirmed-request APDU | 312 | 312 | 0 | 522 | 522 | 0 | 1 / 1 |
| Decode confirmed-request APDU | 222 | 222 | 0 | 239 | 239 | 0 | 0 / 0 |

The release-like matrix is recorded as a no-regression check
(`--aim none`): its job is to show that nothing rose.

## 3. `encode_npdu` without the zero-fill: dropped

`encode_npdu` sizes its output with `BytesMut::resize`, which fills the
whole NPDU's length with zeros, and the view's `encode_into` then writes
over them. `23906182f` removed the fill without `unsafe`: the view encoded
the control information alone into a 48-octet buffer on the stack, and the
output took that and the payload as two appends behind one reservation.
The view's encoder writes only into a slice, so a `BytesMut` can't be
written through it without either the fill or this second copy.

A is `a13238fbb`, B is `23906182f`.

| Workload | Release-like A | B | B - A | Default profile A | B | B - A | Calls A / B |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Encode NPDU, 100-octet payload | 384 | 359 | **-25** | 446 | 447 | **+1** | 1 / 1 |
| Whole ReadProperty request (NPDU, APDU, service) | 1,412 | 1,396 | **-16** | 2,063 | 2,073 | **+10** | 5 / 5 |
| Encode ReadProperty request | 306 | 306 | 0 | 611 | 611 | 0 | 1 / 1 |
| Decode NPDU, 100-octet payload | 153 | 153 | 0 | 154 | 154 | 0 | 0 / 0 |
| Decode ReadProperty request | 180 | 180 | 0 | 169 | 169 | 0 | 0 / 0 |
| Encode confirmed-request APDU | 312 | 312 | 0 | 522 | 522 | 0 | 1 / 1 |
| Decode confirmed-request APDU | 222 | 222 | 0 | 239 | 239 | 0 | 0 / 0 |

- **Release-like, it pays.** The `memset` call is gone (21 instructions for
  a 100-octet NPDU, 12 for the 13-octet one of a ReadProperty request) and
  `encode_npdu` runs 24 fewer, for one more `memcpy` call (20).
- **In the default profile, the second copy costs what the fill did.** The
  calls to `BytesMut::resize` (37), `memset` (21) and `Npdu::encoded_len`
  (22) are gone, 80 instructions. In their place are two more `memcpy`
  calls (33) and 47 more instructions under the `Npdu::encode_into`
  symbol, where callgrind also lists the appends' reservation checks and
  copies in both builds:
  `item3-encode-fill/default-profile/attribution-encode_npdu_100b_payload.md`.
  For the short NPDU of a whole request the fill was only 12 instructions,
  so the same cost leaves it 10 higher.

It rises in one profile, so by the rule it is dropped: `493b8647b` reverts
it, and `encode_npdu` is as on dev. Removing the fill in both profiles
would most likely take an encoder in `bacnet-npdu` that appends to its
output instead of writing into a slice sized beforehand: an addition to
that crate's public API, and outside this issue.

## Round trips

The no-regression check (`--aim none`), release-like: dev `9e6bb0981`
against `493b8647b`, which has items 1 and 2 and not item 3. Figures are
instructions per request. `ct` is Tokio's current-thread runtime, `mt` its
multi-thread runtime with two workers. The floor is the range of the twelve
A and A' values of the same matrix; A and A' were byte-identical.

| Workload | Runtime | A | B | B - A | Floor | Calls A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- |
| ReadProperty | ct | 31,008.8 | 31,009.0 | +0.2 | 15.1 | 18.0001 / 18.0001 | PASS |
| ReadProperty | mt | 32,305.3 | 32,302.3 | -2.9 | 169.1 | 18.0003 / 18.0003 | PASS |
| WriteProperty | ct | 39,918.7 | 39,918.3 | -0.4 | 213.6 | 25.0002 / 25.0002 | PASS |
| WriteProperty | mt | 41,114.7 | 41,133.2 | +18.5 | 247.8 | 25.0003 / 25.0002 | PASS |
| ReadPropertyMultiple | ct | 90,438.8 | 90,435.5 | -3.3 | 83.5 | 139.0004 / 139.0004 | PASS |
| ReadPropertyMultiple | mt | 92,950.6 | 93,059.9 | +109.4 | 689.1 | 139.0005 / 139.0005 | PASS |

**Accepted.** No row rises above its floor, the largest change being 0.2
floors, and no allocator calls are added.

The round trips don't fall either, and nothing here says they should. The
client's and server's network layer reads each frame through the borrowed
view (`bacnet-network`'s `layer_receive.rs`, on the router core's node),
not through `decode_npdu`, so item 1 is not on their path: its callers are
the CLI's and TUI's monitors, test fixtures and code outside this
repository. perf-s1's estimate of about 36 instructions per round trip
assumed one `decode_npdu` at each end. Item 2 changes nothing in a
release-like build.

## Firmware

`check-no-std.sh` and `check-firmware-link.sh` pass on `493b8647b`, with
every image inside its budget. Flash in bytes, before (dev `9e6bb0981`)
and after. RAM is unchanged in every image, and the seven images without
an allocator (mstp, mstp-extended, bip, node, bbmd, router and device) are
the same size to the byte on every target.

| Image | Target | Flash before | After | Change | Budget |
| --- | --- | ---: | ---: | ---: | ---: |
| apdu | thumbv7m | 42,868 | 43,480 | +612 | 47,840 |
| apdu | thumbv6m | 43,796 | 44,520 | +724 | 48,864 |
| apdu | riscv32imc | 50,170 | 50,902 | +732 | 56,000 |
| apdu | thumbv7em | 42,676 | 43,288 | +612 | 47,600 |
| objects | thumbv7m | 144,868 | 144,800 | -68 | 160,816 |
| objects | thumbv7em | 144,984 | 144,916 | -68 | 160,576 |
| objects | riscv32imac | 174,262 | 174,190 | -72 | 193,232 |
| reporting | thumbv7m | 195,840 | 195,776 | -64 | 212,528 |
| reporting | thumbv7em | 196,652 | 196,588 | -64 | 213,088 |
| reporting | riscv32imac | 235,290 | 235,218 | -72 | 255,040 |

**Item 1 costs the apdu image about 560 bytes of flash.** It is the one
image that calls `decode_npdu`, and it now holds the direct parser beside
the view's decoder, which stays for the faults: on thumbv7m, 556 of its
612 bytes come with item 1 (42,868 to 43,424) and 56 with item 2. Item 2
is what makes the objects and reporting images smaller.

## Files

- `item1-direct-decode/`, `item2-bytesmut-sink-write/`, `item3-encode-fill/`:
  for each profile (`release-like/`, `default-profile/`), the matrix's
  `summary.md` and `summary.json` from `perf_counts.py`, the per-function
  and per-line slopes of the workloads the item aims at (`profiles/`), and
  for the default profile the `attribute` output.
- `round-trips/`: the no-regression check's `summary.md`, `summary.json`
  and #1978 item shares (`items.md`).

The summaries' "not acceptable as run" lines only say that a codec-only
matrix has no round trips to judge.

Reproduce (each default-profile run copies `benchmarks/counts/` to a
scratch directory, deletes `lto` and `codegen-units` from its
`[profile.release]` and passes it with `--harness`):

```bash
python3 scripts/perf/perf_counts.py run --baseline 9e6bb0981 --candidate 3b042d0e7 \
  --workloads codec --aim decode_npdu_100b_payload --out /tmp/perf-2334-item1
python3 scripts/perf/perf_counts.py run --baseline 3b042d0e7 --candidate a13238fbb \
  --workloads codec --aim encode_read_property_request,full_stack_encode_rp \
  --harness /tmp/counts-default-profile --out /tmp/perf-2334-item2-default
python3 scripts/perf/perf_counts.py run --baseline a13238fbb --candidate 23906182f \
  --workloads codec --aim encode_npdu_100b_payload,full_stack_encode_rp --out /tmp/perf-2334-item3
python3 scripts/perf/perf_counts.py run --baseline 9e6bb0981 --candidate 493b8647b \
  --workloads rp,wp,rpm --aim none --out /tmp/perf-2334-round-trips
```
