# bacnet-mstp-core

The MS/TP master node of Clause 9 of ASHRAE 135 as a portable core, for
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet). It sits above
`bacnet-mstp-codec`, which has the wire format, and below a UART or a host
serial port.

- `#![no_std]`, no allocator, no dependencies besides `bacnet-mstp-codec`
  and `bacnet-time`. The codec is re-exported as `bacnet_mstp_core::codec`,
  since the API speaks its types.
- The core reads no clock and owns no serial port. You feed it what the
  line does, each event stamped with an `Instant`, and carry out the
  actions it returns: switch the transceiver, send a frame, deliver an
  NPDU. The `Instant` is `bacnet-time`'s, in nanoseconds, re-exported here,
  the same type `bacnet-bip-core` and `bacnet-router-core` take, so
  firmware feeds them one clock. A `now` earlier than the latest one passed
  in is taken as that latest one: time never runs backwards, so a clock
  that steps back holds the timers still instead of failing.
- Its large buffers are yours: the receive storage, the `ActionBuffers`
  and the queues, each built by a `const fn` of zeros, so firmware keeps
  them in `.bss` and builds the core, a few hundred octets, on its stack.
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
  with a starvation limit: `HeaplessQueues`, in arrays sized at compile
  time, `AllocQueues` with the `alloc` feature, or a `QueueStorage` of your
  own, whose methods are part of the crate's semver.

The Tokio MS/TP transport in `bacnet-transport` and the MS/TP simulator in
`bacnet-mstp-sim` both drive this core.

## Features

- `alloc`: `AllocQueues`, which reserve their capacity when built.

None is on by default, and none needs `std`.

## Example

```rust
use bacnet_mstp_core::codec::{decode_frame, Frame, FrameType, MAX_STANDARD_FRAME_LENGTH};
use bacnet_mstp_core::{
    Action, ActionBuffers, Direction, Duration, HeaplessQueues, Instant, MasterCore,
    MstpCoreConfig, NetworkPriority,
};

// Station 1 of masters 0 to 3, at 9600 baud.
let mut config = MstpCoreConfig::STANDARD;
config.station = 1;
config.max_master = 3;

let mut rx = [0u8; MAX_STANDARD_FRAME_LENGTH];
let mut actions: ActionBuffers<1> = ActionBuffers::new();
let queues: HeaplessQueues<4, 2> = HeaplessQueues::new();
let mut core = MasterCore::new(config, queues, &mut rx, &mut actions, Instant::ZERO).unwrap();

// An NPDU to send when the token comes.
let npdu = [0x01, 0x00, 0x10, 0x08];
core.enqueue_npdu(2, &npdu, NetworkPriority::Normal, Instant::ZERO).unwrap();

// Station 2 passes the token: its octets, as a serial read returns them.
let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
let length = Frame::new(FrameType::TOKEN, 1, 2, &[]).encode_into(&mut wire).unwrap();
let heard = Instant::from_millis(10);
for &octet in &wire[..length] {
    core.on_octet(octet, heard).unwrap();
}

// The NPDU goes out once the line has been quiet for T_turnaround.
let now = core.next_deadline().unwrap();
assert_eq!(now, heard + core.config().t_turnaround);
core.on_deadline(now).unwrap();
assert_eq!(core.poll_action(), Action::SetDirection(Direction::Transmit));
let Action::Transmit { id, frame } = core.poll_action() else {
    panic!("a frame to send");
};
let (sent, _) = decode_frame(frame).unwrap();
assert_eq!(sent.header.frame_type, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
assert_eq!((sent.header.destination, sent.data), (2, &npdu[..]));

// Drive the line only once the core confirms the start, then report the
// frame's last stop bit.
core.on_transmit_started(id, now).unwrap();
core.on_transmit_complete(id, now + Duration::from_millis(20)).unwrap();

let counters = core.take_counters();
assert_eq!((counters.frames_received, counters.frames_transmitted), (1, 1));
```

## License

MIT
