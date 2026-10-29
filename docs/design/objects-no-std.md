# Design: bacnet-objects without std (#1615)

Status: **done** in the PR that adds this note. It settles OD-31 (how the
B/IP Network Port support reaches a no_std build) as option (a), reshape.

## 1. Goal

`bacnet-objects` builds on a bare-metal target with a global allocator:

```bash
cargo check -p bacnet-objects --no-default-features --features alloc \
  --target thumbv7em-none-eabihf
```

The hosted build keeps its API and behaviour. `std` stays a default feature
and brings back everything below that needs it.

The crate needs an allocator in every build, through `std` or `alloc`:
`ObjectDatabase` boxes every object,
so a heap-free device answers from a static table instead (#1771). It also
needs atomic compare-and-swap, since objects share state through
`alloc::sync::Arc`, which `alloc` leaves out on targets without it. So ARMv6-M
(`thumbv6m-none-eabi`) is out, and `scripts/ci/check-no-std.sh` checks the
crate on the targets that have compare-and-swap (`thumbv7em-none-eabihf`,
`riscv32imac-unknown-none-elf`). A build with neither feature, or for a target
without compare-and-swap, stops with a `compile_error!` that says which.

## 2. Audit

Every std dependency the crate had at dev `53a65d44`, where it lives, its
size (S: a few lines; M: one module's design; L: several modules) and what
the port does with it.

| Dependency | Where | Size | Plan |
|---|---|---|---|
| std prelude names (`String`, `Vec`, `Box`, `ToString`, `ToOwned`) | about 110 library files | S each | `use crate::prelude::*;` in the files that need it: those names from `alloc` |
| `std::` paths that `core` or `alloc` have (`fmt`, `mem`, `ops`, `borrow::Cow`, `sync::Arc`/`Weak`, `sync::atomic`, `time::Duration`, `collections::{VecDeque, BTreeMap}`, `net`, `error`, ...) | about 150 library files | S | Rewritten to the `core::`/`alloc::` path. Same types, so nothing changes for a hosted caller |
| `HashMap`, `HashSet` | 8 library files (`ObjectDatabase`'s indexes, trend polling, Device properties, Access Rights evaluation, schedule, elevator and receipt duplicate checks), and the metadata test modules | S | `crate::collections`: std's with `std`, hashbrown's (foldhash hasher) without. Both are private: no public signature names a map, so the feature can't change a public type. Neither order is fixed, so no output depended on it before, and the hosted build keeps std's maps |
| `std::sync::Mutex` | `EventSequence`, Audit Reporter status and association, the forwarder's confirmed lists, the log store's pending snapshot, the save writer's `Event` | S | `crate::sync::Mutex`: std's with `std`, a spinning_top spin lock without. Its `lock()` hands back the guard, so the 27 `lock().unwrap()` sites (Audit Reporter status 14, Reporter delay 6, association 3, EventSequence 4) now go on after a poisoned lock as the writer's sites already did; every value they guard is whole between statements. A panic inside an EventSequence transaction closure now leaves the number unconsumed, where before it poisoned every later call |
| `AtomicU64` | Command/Channel run generation, two save-failure counters, the log store's generation | S | `crate::sync::AtomicU64`: core's on a target with 64-bit atomics, a `Mutex<u64>` with the same calls on one without (32-bit Arm and RISC-V). It keys on the target, not on `std` |
| `f64::trunc`, `ceil`, `round`, `hypot` | Pulse Converter adjust, Channel coercion, COV increment, colour transitions | S | `crate::float`: std's with `std`, libm's without |
| `catch_unwind` | Life Safety reset executor | S | `crate::catch_panic`: catches with `std`; without it a panic goes on up |
| `tracing` | 16 files (save failures, Event Log refusals, trend polling) | S | `log_warn!` / `log_debug!`: tracing with `std`, nothing without. tracing-core needs atomics the no_std crates don't assume, and `check-no-std.sh` keeps tracing out of their graphs. The macros still borrow the variables a line names, so no binding goes unused |
| The save writer's thread (`std::thread`, `Condvar`, `OnceLock`, `JoinHandle`, `catch_unwind`) | `durable.rs` | M | The threaded writer moves to `durable/thread.rs`, unchanged, behind `std`. Without std, `durable/inline.rs` runs each save on the caller's thread before `submit` returns (section 3) |
| `std::time::Instant` (staged-write lifetime) | `durable.rs`, `durable/staged.rs`, the forwarder's saving | S | Behind `std`. Without a clock, a staged write expires only through the operation task's monotonic `Duration` (`StagedSaves::expire`), which every build already has; `SaveTicket::outlived` says never |
| File backends (`std::fs`, `std::io`, `std::path`, `sync_parent_dir`) | `durable/file.rs`, `log_persistence/file.rs`, the `File*Persistence` types in six `persistence.rs` files | M | Behind `std`, with the imports and constants only they use. The persistence traits and snapshots stay in every build, so a device keeps state in its own flash. The snapshot codecs only the file backends call go unused without std |
| `libc` | `durable.rs` (`ENOTSUP` and friends for a directory sync) | S | Optional, turned on by `std` |
| `bip_port` (`Arc<dyn BbmdTables + Send + Sync>`) | `bacnet-types`; used by `database/network_port.rs`, `network_port/{bip_mode,registration,pending,multicast}.rs`, `device_identity` | M | OD-31, section 4 |
| Dependencies: `bacnet-types`, `bacnet-encoding` and `bytes` with `std` hard-wired | `Cargo.toml` | S | Their `std` features now come from this crate's `std`; `alloc` turns on theirs. `std` doesn't turn on `alloc`, so a hosted build compiles none of hashbrown, spinning_top or libm |

No file I/O, thread or clock is left in the no_std build, and nothing in its
graph has a `std` feature on or picks an atomics backend
(`check-no-std.sh` checks both).

## 3. Saving without threads

The objects that keep state in storage (Audit Log, Notification Forwarder,
Notification Class, Access Rights, Color, Color Temperature, Object Profile
tags and the three logs) queue each save on a `SaveWriter`, and the hosted
writer runs it on a thread of its own so the server can wait for it with the
database guard dropped (#1270).

Without std there is nothing to hand a save to, so the inline writer makes
the storage call where it is queued. It has the hosted writer's interface,
so the objects are the same in both builds:

- `submit` saves, reports to the done hook, drops the snapshot and completes
  the ticket, in the hosted writer's order, before it returns. Every
  `SaveWait` is therefore over by the time anyone holds it, and staging still
  works: a staged save has already landed when the request comes back to
  take it.
- `submit_coalescing` saves at once; nothing waits, so nothing is queued to
  replace.
- `idle` is over, and `wait_idle` returns at once.
- A storage call runs under whatever the caller holds, the database guard
  included. On a device that is the expected trade: storage there is a flash
  write the application controls. No done hook takes a lock that the code
  queuing the save holds, so the spin lock never re-enters.
- A panic in a storage call is not caught (only std can), where the hosted
  writer fails that one save.

`SaveWait::block` spins without std. A save's wait never spins, since it is
over already, but a wait for another request's staged write (`StageStep::Busy`)
spins until that request releases it, forever on a single-threaded executor.
So without std an application awaits such a wait and never blocks on it; a
debug build panics if it does.

Application persistence code runs under the database guard without std:
`*Persistence::save` and `commit` are called from inside the object call
that queued the save. A persistence implementation must therefore never
re-enter the database, or it deadlocks on the guard its caller holds.

## 4. OD-31: the B/IP port contract

`bacnet_types::bip_port` lends a transport's BBMD tables, foreign-device
registration and multicast group to the Network Port object as shared live
views (`Arc<dyn BbmdTables>`, and so on). L02 put it behind `std` in 0.13,
because the shared `Arc` needs compare-and-swap.

Options:

- (a) Reshape: build the module with `alloc` on a target with compare-and-swap,
  `#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]`, with its `Arc`
  and `Vec` from `alloc`.
- (b) Gate: keep the module behind `std`, and put every Network Port path
  that names it (the B/IP mode rows, registration, pending changes,
  multicast, the database's reserve and publish calls, `DeviceIdentity`'s
  B/IP helpers) behind `std` in `bacnet-objects` too.

Decision: **(a)**. The module needed nothing from std but the `Arc` path:
its traits already returned `Vec`s and borrowed slices. The hosted API is
the same type at the same path, so nothing changes for a hosted caller, and
`bacnet-objects` needs no cfg in any of its Network Port code. A no_std
B/IP device (`bacnet-bip-core` runs one without std) can lend its BBMD
tables to the Network Port object the same way the Tokio transport does.
Option (b) would have spread `cfg(feature = "std")` through five Network
Port modules and two public `DeviceIdentity` methods, and taken the B/IP
rows away from exactly the devices `bacnet-bip-core` serves.

ARMv6-M still goes without the module, as it goes without `bacnet-objects`.

## 5. Checks

- `scripts/ci/check-no-std.sh` lints `bacnet-objects` with `alloc` on the
  targets with compare-and-swap, denying warnings, and checks its graph:
  no `std` feature, no tracing, no atomics backend.
- CI's alloc-only clippy (all targets) and test runs cover `bacnet-objects`
  too, beside the lower crates. Those tests link std but build the crate
  without its `std` feature, so they run the inline writer, the spin lock,
  hashbrown's maps and libm. The tests that need a thread or a file (the
  threaded writer's own tests, the file backends, the staging tests that
  hold a save open on another thread, and the executor-panic check) stay
  behind `std`.
- `scripts/ci/check-default-features.sh` runs clippy and rustdoc for the
  crate with `alloc` alone, so a doc link to a std-only item fails there.
