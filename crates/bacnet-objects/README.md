# bacnet-objects

Build BACnet object state with [`traits::BACnetObject`] and
[`database::ObjectDatabase`] in the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. The database
owns registered objects and supports identifier-based lookup; object modules
provide their property implementations.

The crate has no Tokio dependency. Using it alone does not start a
transport, a server executor or event/COV delivery.

## Features

| Profile | Features | What it has |
| --- | --- | --- |
| Hosted | `std` (default) | Everything, including the file persistence backends (`FileAuditLogPersistence`, `FileLogPersistence` and the others), a writer thread per saving object, and tracing. |
| `no_std` with a heap | `alloc` (without `std`) | Every object type, the database and the persistence traits, for a target with a global allocator and atomic compare-and-swap (#1615). A save runs on the caller's thread, and the application supplies the storage; `snapshot` gives it the octets the file backends write (#2047). |

Two capability features (#2215) let a firmware image drop code it never
uses. `std` turns both on, so a hosted build has them; a `no_std` build with
`alloc` alone has neither until it names them.

| Feature | Off, the built-in objects lend nothing through | What drops out | Objects image, flash saved (thumbv7m / thumbv7em / riscv32imac) |
| --- | --- | --- | --- |
| `event-reporting` | `event_reporting()`, `event_reporting_mut()`, `enrollment_eval()`, `enrollment_eval_mut()` | Intrinsic reporting's evaluation and commits, alarm acknowledgment, the Event and Alert Enrollments' transitions and the logs' buffer-ready reports. The event properties still read and write. | 13,472 / 13,328 / 16,836 octets |
| `durable-writes` | `durable_writes()` | Staged saves, which a host runs off the database lock. A write to an object with storage still saves, on the writer's own path, and with the feature off the built-in objects do not implement `DurableWrites`, so a caller that stages needs it on. | 3,400 / 3,708 / 4,544 octets |

Measured on the objects image in `firmware/link` (a Device, two Analog
Inputs and two Binary Values behind `bacnet-device-core`'s responder): each
figure is that feature on alone, with the other off, against both off. With
both on the image is 38,384 / 38,032 / 48,476 octets of flash larger than
with both off, more than the two figures add up to, because with both on
the reporting store's staged-save check links the parameter encoders. The object types, their
properties and the capability traits are the same with either feature on or
off, so turning one on never breaks another crate's build.

There is no build with neither feature: the database boxes every object, so a
heap-free device answers from a static table instead. ARMv6-M
(`thumbv6m-none-eabi`) lacks compare-and-swap, which `alloc::sync::Arc`
needs, so the crate doesn't build there.
[The design note](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/design/objects-no-std.md)
has the details.

## Initialize and read an object

Place an application directory beside a checkout named `rusty-bacnet`, then
add this dependency block to the application's `Cargo.toml`. These are
development-source paths, not a claim that the shown APIs are published;
adjust the paths if your directories differ.

```toml
[dependencies]
bacnet-objects = { path = "../rusty-bacnet/crates/bacnet-objects" }
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types" }
```

Save this complete program as `src/main.rs`. It initializes an unattached
Analog Input, transfers it to the database, then checks the stored identity,
Present_Value and a missing-object lookup.

```rust
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

fn main() -> Result<(), bacnet_types::error::Error> {
    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7)?;
    let mut analog = AnalogInputObject::new(
        7,
        "supply-temperature",
        EngineeringUnits::DEGREES_CELSIUS,
    )?;
    analog.set_present_value(42.0);
    let mut database = ObjectDatabase::new();
    database.add(Box::new(analog))?;

    let stored = database.get(&object).expect("the object was registered");
    assert_eq!(stored.object_identifier(), object);
    assert_eq!(stored.object_name(), "supply-temperature");
    assert_eq!(
        stored.read_property(PropertyIdentifier::PRESENT_VALUE, None)?,
        PropertyValue::Real(42.0),
    );
    let absent = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 8)?;
    assert!(database.get(&absent).is_none());
    Ok(())
}
```

The direct setter above initializes an object before ownership transfer.
`AnalogInputObject::set_present_value` bypasses running-server ownership,
validation, intrinsic-event and COV processing. For live server updates, use
`BACnetServer::set_present_value_local` and its documented policy instead of
mutating a database object directly.

## Device identity and executor ownership

Use [`DeviceIdentity`] and [`device_identity::build_database_with_extra`] when
assembling a server database. The helper installs the Device and configured
Network Ports, and seeds the Device's Object_List with the extra application
objects. Adding a standalone analog object alone does not create a Device.
[`points::build_database`] builds the same database from a plain-data point
list (`bacnet_types::points`), checking every identifier and name first.

Objects read through a raw database return standalone state. A running
`BACnetServer` supplies live executor-owned Device properties and lists through
`read_local`; its shared database handle is not an equivalent live view. Never
hold a database guard across network operations or shutdown. Durable objects
can queue saves, so their final drop or the server's explicit stop may wait on
storage; this example uses only an in-memory Analog Input.

The example establishes local object initialization, lookup and property
access. It does not establish transport operation, event delivery, physical
equipment behavior, timing or standards conformance. Consult each object's
property and service documentation for its supported behavior.

## Documentation profile

This README is included as the crate landing page and the program runs as a
doctest. The docs.rs profile disables defaults, explicitly selects `std` and
targets `x86_64-unknown-linux-gnu`, so it shows the hosted API, the file
persistence backends included; it does not render the `alloc`-only build, and a
local render does not establish a docs.rs deployment.
