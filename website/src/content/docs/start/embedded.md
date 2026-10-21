---
title: "Run on a microcontroller"
description: "Which no_std crates an MS/TP device, a B/IP device or a router needs, the shared clock, measured sizes, and the rules an adapter follows."
---

The portable crates run on a microcontroller with no operating system and no heap. Each one is a sans-IO core: it reads no clock and owns no UART or socket. Your firmware feeds it what the line does, stamped with the time, and carries out the actions it hands back. This page says which crates a product needs, how they share time, how much memory they take, and what the adapter between them and your hardware must do.

This page describes v0.14.0, the first release with the allocation-free profile of `bacnet-types`, `bacnet-encoding` and `bacnet-services`, the `bacnet-npdu` crate, the MS/TP storage bundle and `bacnet_time::earliest` (a new `bacnet-time` version that only adds API). The firmware images, the `device` image among them, live in the repository, not on crates.io. Nothing here is a hardware qualification: the images are measured and checked natively, and run under emulation by hand.

## Crates by product

Every crate below is `no_std` and needs no allocator. The cores re-export what their API speaks, so you can often depend on one crate: `bacnet-mstp-core` re-exports its codec as `codec` and `bacnet-npdu` as `npdu`, and `bacnet-router-core` always re-exports `npdu`, and `bip_core` and `mstp_core` with its `bip` and `mstp` features. Depend on the application crates with `default-features = false` to get their allocation-free profile.

| Product | Data link | Network layer | Application layer | Firmware image that measures it |
| --- | --- | --- | --- | --- |
| MS/TP device | `bacnet-mstp-core` (with `bacnet-mstp-codec`) | `bacnet-npdu` | `bacnet-types`, `bacnet-encoding` and `bacnet-services`, no default features | `device` |
| B/IP device | `bacnet-bip-core`'s `NodePort` (with `bacnet-bip-codec`) | `bacnet-npdu` | the same three crates | `node`, for the link alone |
| B/IP BBMD | `bacnet-bip-core`'s `BbmdPort` and `Tables` | `bacnet-npdu` | the same three crates, when it hosts a Device object | `bbmd`, for the link alone |
| Router between B/IP and MS/TP | `bacnet-router-core` with `bip` and `mstp` | `bacnet-router-core` | the same three crates, for its Device object | `router`, for routing alone |

A router is also a device: Clause 6.6 of the standard holds it to the device requirements of Clause 22.1.5, so a product router adds a Device object and the ReadProperty service to the routing the `router` image measures. B/IPv6, BACnet/SC and Ethernet stay hosted, on the standard library.

The `device` image in `firmware/link` is the reference for an MS/TP device: it answers Who-Is with an I-Am and ReadProperty for a Device object and an Analog Input, with the Error, Reject or Abort each refused request needs. Its source, `firmware/link/src/device/app.rs`, is the application part to copy. It is a starting point, not a conformant Device as it stands: it lacks Property_List and other properties Clause 12.11 requires of a Device object, and its Vendor_Identifier of 0 is a placeholder. A product fills these in with its own.

## One clock for every core

All the cores take `bacnet-time`'s `Instant` and `Duration`: nanoseconds on a 64-bit unsigned integer, from an epoch you choose. That is about 584 years of range. Build an `Instant` from your timer with `Instant::from_micros` or `Instant::from_millis`.

- **Feed one clock to every core.** Read the timer once per event and pass that `now` to each call. The cores compare deadlines across calls, so two clocks that drift apart confuse them.
- **Time never runs backwards.** A `now` earlier than the latest one a core has seen is taken as that latest one, so a clock that steps back holds the timers still instead of failing.
- **Merge deadlines with `bacnet_time::earliest`.** Each core reports its next deadline as an optional `Instant`, `None` when nothing is due. `earliest` skips the `None`s. `Option::min` does not: it picks `None`, and a timer merged that way never fires.
- **Sleep until the merged deadline or the next received octet,** then call `on_deadline` on each core whose deadline came, and poll its actions.
- **No 64-bit division.** The cores and `bacnet-time` avoid it, so a 32-bit part doesn't link the roughly one-kilobyte division routine.
- On a host, the `std` feature of `bacnet-time` adds `HostClock`, which maps the standard library's monotonic clock onto the same timeline.

## Storage sizing

Each core's large buffers are yours, and a `const fn` builds them from zeros, so firmware keeps them in a static in the zero-initialized section, `.bss`, costing no flash. Build the core itself on the stack over that static; it borrows the buffers and is a few hundred bytes. For MS/TP, one `StandardStorage` bundle holds the receive buffer and the queue and delivery slots, their counts given as its three type parameters; `ExtendedStorage` is the same, sized for extended frames.

CI links each firmware image for four targets and checks it against a budget in `firmware/link/size-budget.toml`. These sizes were measured for the Cortex-M0+ target, `thumbv6m-none-eabi`, with the size-optimized `firmware` profile. Flash is code, read-only data and the vector table. Static RAM is initialized and zeroed data. The stack column is the budget in `firmware/link/stack-budget.toml` for the BBC micro:bit board under emulation (the router's is the LM3S6965 board's, since its statics don't leave a 16-kibibyte part room for its stack); each budget is the deepest stack measured plus a quarter. Each image links a test scenario as well as its stack, so compare images with care.

| Image | What it links | Flash (bytes) | Static RAM (bytes) | Stack budget (bytes) |
| --- | --- | --- | --- | --- |
| `mstp` | MS/TP codec and master core, standard frames | 18,180 | 5,528 | 2,560 |
| `mstp-extended` | the same with extended frames | 17,048 | 7,696 | 3,776 |
| `node` | B/IP foreign-device node and its port | 15,128 | 240 | 6,816 |
| `bbmd` | a BBMD's port and tables | 12,376 | 240 | 2,976 |
| `device` | a whole MS/TP device with Who-Is and ReadProperty | 29,876 | 3,496 | 3,328 |
| `router` | a B/IP-to-MS/TP router whose B/IP port is the BBMD | 43,320 | 15,536 | 5,584 |

What drives the RAM:

- An MS/TP standard frame carries up to 501 bytes of NPDU. The `device` image's 3,496 bytes of static RAM are mostly its storage bundle: one unicast slot, two broadcast slots and one delivery slot, about 3.3 kilobytes, plus 88 bytes of application scratch and an 88-byte buffer for the answer's NPDU.
- An extended frame carries up to 1,497 bytes. Clause 9.3 asks them of a router's MS/TP port, and their buffers are most of the router's 15.5 kilobytes of static RAM. A device needs them only to take NPDUs longer than 501 bytes.
- Each more queue slot costs about one NPDU: 501 bytes for standard frames, 1,497 for extended.
- The `device` image links no formatting code (`core::fmt`), no 64-bit division and no `memmove`; the check counts each and fails if one appears.

## Adapter rules

The adapter is the code between a core and your hardware. The rules below keep the cores honest about the line; the crate documentation of `bacnet-mstp-core` and `bacnet-bip-core` gives them in full.

- **Poll after every call.** After each call into a core, poll its actions until it reports idle, and carry each one out before the next event.
- **A frame's octets come with its start.** An MS/TP `Transmit` action names a frame. Right before you enable the line driver, call `on_transmit_started`: it returns the octets only if it confirms the start, so the line is never driven for a frame the core took back. A start within the turnaround time of line activity is refused and the frame comes back later as a new action. Call `on_transmit_complete` when the last stop bit has left.
- **Echo begins at the confirmed start.** Until then keep the driver off and the receiver on, and feed the core every octet and receive error, draining the receive buffer right before the start call. What you hear between the start and the completion is your own echo, which the core ignores.
- **Inputs can't fail.** The `on_*` calls that report the line never return an error. An input that breaks the contract is ignored and counted in the core's adapter-error counter; watch it in testing. Only building a core, a transmit start and a request's answer return errors.
- **Answer a request in its reply window.** A data-expecting-reply frame comes with a request id. Hand the core your answer with `complete_request`; if the answer isn't ready, the core sends Reply Postponed for you, and if there is none, abandon the request so the core stops waiting.
- **Report where each datagram went.** A B/IP adapter tells the core, for each datagram it reads, whether it came to this node alone or to the subnet's broadcast address. An IP stack that can't tell passes `None`, and the core drops the datagram rather than guess.
- **Counters drain.** `take_counters` returns 32-bit counts since the last take and starts them again from zero. Add each take to totals of your own, as wide as you need.

Two host-runnable examples follow these rules and compile in CI: `crates/bacnet-mstp-core/examples/node.rs`, two master nodes on a simulated line, and `crates/bacnet-router-core/examples/bip_mstp_router.rs`, a router whose B/IP port is the BBMD, in the main loop firmware runs.

```sh
cargo run -p bacnet-mstp-core --example node
cargo run -p bacnet-router-core --features bip,mstp --example bip_mstp_router
```

## What still needs an allocator

Without `alloc`, the application crates have the enums, the object identifier, the error type, tag and primitive encoding and decoding, borrowed views of strings and application-tagged values, the APDU header (with Error, Reject and Abort), and Who-Is, I-Am and ReadProperty, request and acknowledgement. Everything else in them needs `alloc` and the `bytes` crate:

- the other services, such as WriteProperty, ReadPropertyMultiple and change-of-value subscriptions;
- the owned APDU and NPDU types, and segmentation;
- owned property values (`PropertyValue`) and the constructed types;
- every encoder that writes into a growable `BytesMut` buffer.

`bacnet-objects`, with its object database, and the Tokio client, server and transports need the standard library. Firmware that wants them today builds the application crates with `alloc` and supplies a global allocator; the `apdu` firmware image does that with a one-kilobyte heap.

## Sources

[firmware/link](https://github.com/jscott3201/rusty-bacnet/tree/dev/firmware/link) (the images, their scenarios and budgets) · [Rust API notes](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/rust-api.md) · [CI's firmware link test](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/ci.md#firmware-link-test).
