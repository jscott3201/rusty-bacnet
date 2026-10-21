# bacnet-objects

Build BACnet object state with [`traits::BACnetObject`] and
[`database::ObjectDatabase`] in the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. The database
owns registered objects and supports identifier-based lookup; object modules
provide their property implementations.

This hosted crate requires `std` and allocating storage. It has no public
features and no Tokio dependency. Using it alone does not start a transport,
a server executor or event/COV delivery, and does not provide an alloc-only or
no-heap firmware profile.

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
doctest. The docs.rs profile explicitly selects no features, disables defaults
and targets `x86_64-unknown-linux-gnu`. Those metadata settings do not make this
crate `no_std`, and a local render does not establish a docs.rs deployment.
