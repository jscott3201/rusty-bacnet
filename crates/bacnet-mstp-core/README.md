# bacnet-mstp-core

The MS/TP master node of Clause 9 of ASHRAE 135 as a portable core, for
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet). It sits above
`bacnet-mstp-codec`, which has the wire format, and below a UART or a host
serial port.

- `#![no_std]`, no allocator, no dependencies besides `bacnet-mstp-codec`,
  `bacnet-npdu` and `bacnet-time`. The codec is re-exported as
  `bacnet_mstp_core::codec` and the network-layer vocabulary as
  `bacnet_mstp_core::npdu`, since the API speaks their types (an NPDU's
  network priority is `npdu::NetworkPriority`).
- The core reads no clock and owns no serial port. You feed it what the
  line does, each event stamped with an `Instant`, and carry out the
  actions it returns: switch the transceiver, send a frame, deliver an
  NPDU. The `Instant` is `bacnet-time`'s, in nanoseconds, re-exported here,
  the same type `bacnet-bip-core` and `bacnet-router-core` take, so
  firmware feeds them one clock. A `now` earlier than the latest one passed
  in is taken as that latest one: time never runs backwards, so a clock
  that steps back holds the timers still instead of failing.
- Its large buffers are yours, in one bundle: a `StandardStorage` or
  `ExtendedStorage` holds the receive storage, the queues and the
  `ActionBuffers`, sized together and built by a `const fn` of zeros, so
  firmware keeps it in `.bss` and builds the core, a few hundred octets, on
  its stack.
- Line events go in through `on_*` calls that cannot fail; what breaks the
  contract is counted. Only construction, a transmit start and a request's
  answer return an error, each its own type.
- Extended frames (types 32 and 33, NPDUs of up to 1497 octets) sit behind
  one flag, `MstpCoreConfig::extended_frames`. Buffer sizes are const
  generics with standard defaults, so an image without them pays nothing.
- Counters drain: `take_counters()` returns the `u32` counts since the last
  take, and you add them to totals of your own.
- The types that may grow are `#[non_exhaustive]`: start a `MstpCoreConfig`
  from `MstpCoreConfig::STANDARD` or `for_baud_rate` and set the fields to
  change, and give a `match` on an error or a refusal a wildcard arm. The
  values Clause 9.5.3 fixes are constants (`N_POLL`, `T_NO_TOKEN` and the
  rest), not settings.

## What it has

- `MasterCore`, the master node: token passing and the search for a
  successor, maintenance polls, a lost token and its regeneration,
  answering a data request (or Reply Postponed when the application is
  slow), Test_Request, and the turnaround before every frame. It checks
  its configuration against the Clause 9.5.3 bounds when it is built.
- Two input paths: every octet through the core's own stream decoder, or
  whole frames from a backend that assembles them in hardware.
- Transmit queues for unicast and broadcast, served by network priority
  with a starvation limit: `FixedQueues`, in arrays sized at compile
  time, `AllocQueues` with the `alloc` feature, or a `QueueStorage` of your
  own, whose methods are part of the crate's semver.

The Tokio MS/TP transport in `bacnet-transport` and the MS/TP simulator in
`bacnet-mstp-sim` both drive this core.

## Features

- `alloc`: `AllocQueues`, which reserve their capacity when built.
- `inspect`: a hidden view of a frame handed over and not yet started, a
  test and fuzz hook outside the crate's semver. Adapters never need it.

None is on by default, and none needs `std`.

## Example

```rust
use bacnet_mstp_core::codec::{decode_frame, Frame, FrameType, MAX_STANDARD_FRAME_LENGTH};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, Direction, Duration, Instant, MasterCore, MstpCoreConfig, StandardStorage,
};

// Station 1 of masters 0 to 3, at 9600 baud.
let mut config = MstpCoreConfig::STANDARD;
config.station = 1;
config.max_master = 3;

// Receive storage, four unicast and two broadcast queue slots, and one
// delivery slot, all zeroed: on firmware, a static in `.bss`.
let mut storage: StandardStorage<4, 2, 1> = StandardStorage::new();
let mut core = MasterCore::new(config, &mut storage, Instant::ZERO).unwrap();

// An NPDU to send when the token comes.
let npdu = [0x01, 0x00, 0x10, 0x08];
core.enqueue_npdu(2, &npdu, NetworkPriority::Normal, Instant::ZERO).unwrap();

// Station 2 passes the token: its octets, as a serial read returns them.
let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
let length = Frame::new(FrameType::TOKEN, 1, 2, &[]).encode_into(&mut wire).unwrap();
let heard = Instant::from_millis(10);
for &octet in &wire[..length] {
    core.on_octet(octet, heard);
}

// The NPDU goes out once the line has been quiet for T_turnaround.
let now = core.next_deadline().unwrap();
assert_eq!(now, heard + core.config().t_turnaround);
core.on_deadline(now);
assert_eq!(core.poll_action(), Action::SetDirection(Direction::Transmit));
let Action::Transmit { id, header, .. } = core.poll_action() else {
    panic!("a frame to send");
};
assert_eq!(header.frame_type, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);

// The octets come only with a confirmed start, right before the line
// driver goes on, so a refused start has nothing to send. Then report the
// frame's last stop bit.
let frame = core.on_transmit_started(id, now).unwrap();
let (sent, _) = decode_frame(frame).unwrap();
assert_eq!((sent.header.destination, sent.data), (2, &npdu[..]));
core.on_transmit_complete(id, now + Duration::from_millis(20));

let counters = core.take_counters();
assert_eq!((counters.frames_received, counters.frames_transmitted), (1, 1));
```

`examples/node.rs` puts two master nodes on a simulated line with a virtual
clock, each driven the way a UART adapter drives the core, and prints the
token ring forming and a request answered in its reply window:
`cargo run -p bacnet-mstp-core --example node`.

## License

MIT
