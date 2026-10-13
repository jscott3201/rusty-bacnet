# bacnet-time

The time the portable cores of
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet) share: a monotonic
`Instant` and a `Duration`, both counted in nanoseconds on a `u64`.
`bacnet-mstp-core`, `bacnet-bip-core` and `bacnet-router-core` take it, so
firmware that runs several of them feeds them one clock.

- `#![no_std]`, no allocator, no dependencies, no features.
- The epoch is yours: the cores read no clock, and each call that depends on
  time takes `now`. A `u64` of nanoseconds lasts about 584 years.
- `+` and `-` saturate, and every operation has a `checked_*` form that
  returns `None` instead. Nothing panics.
- Unit constructors (`from_micros`, `from_millis`, `from_secs`) are `const`
  and saturate; unit accessors (`as_micros`, `as_millis`, `as_secs`) round
  toward zero.
- Conversions with `core::time::Duration`, so a host maps its monotonic
  clock onto the timeline exactly: `From` into it, and `TryFrom` back,
  which refuses a length past about 584 years, or the `const`
  `Duration::saturating_from_core`, which clamps it.
- `const` `min` and `max`, and a `Debug` with the unit:
  `Duration(1500ns)`.
- No 64-bit division: on a 32-bit target, where dividing a `u64` links a
  routine of about 1 KB, the accessors, `checked_rem`, the conversion to
  `core::time::Duration` and `Debug` use binary long division.

## Example

```rust
use bacnet_time::{Duration, Instant};

// Firmware with a millisecond tick.
let now = Instant::from_millis(1_250);
let deadline = now + Duration::from_millis(500);
assert_eq!(deadline.saturating_duration_since(now).as_millis(), 500);

// Saturation, and the checked forms.
assert_eq!(Instant::ZERO - Duration::from_secs(1), Instant::ZERO);
assert_eq!(Instant::ZERO.checked_sub(Duration::from_secs(1)), None);
assert_eq!(now - deadline, Duration::ZERO);

// A host's monotonic clock, through core::time::Duration.
let elapsed = core::time::Duration::from_micros(2_500);
assert_eq!(Duration::try_from(elapsed), Ok(Duration::from_micros(2_500)));
assert_eq!(Duration::saturating_from_core(core::time::Duration::MAX), Duration::MAX);
assert_eq!(
    core::time::Duration::from(Duration::from_nanos(1_500)),
    core::time::Duration::from_nanos(1_500)
);
assert_eq!(format!("{:?}", Duration::from_nanos(1_500)), "Duration(1500ns)");
```

## License

MIT
