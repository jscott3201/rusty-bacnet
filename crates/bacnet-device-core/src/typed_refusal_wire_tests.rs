//! The refusals bacnet-objects and this crate report with the typed kinds
//! of #1833, in place of the untyped text #1833 then removed, go
//! out on the wire as that text did: an Error PDU with SERVICES / OTHER, the
//! plain one and each Clause 21 production alike. Each refusal comes from a
//! real site, so a site moved to a decoding error, which a request decode
//! turns into a Reject, or to an error that names its own class and code,
//! fails here.

use crate::cov::{CovPolicy, CovRecipient, flags::PreparedFlags, validate_flags};
use crate::read::ReadPropertyMultipleBudget;
use crate::reply::error_apdu;
use crate::responder::{Limits, Responder};
use bacnet_encoding::apdu::encode_apdu;
use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::network_port::{BipPortConfig, NetworkPortObject};
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, ErrorClass, ErrorCode, ObjectType,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

fn wire(service: ConfirmedServiceChoice, error: &Error) -> Vec<u8> {
    let mut out = BytesMut::new();
    encode_apdu(&mut out, &error_apdu(9, service, error)).unwrap();
    out.to_vec()
}

/// The kind a refusal reports, by its variant.
fn kind(error: &Error) -> &'static str {
    match error {
        Error::State(..) => "State",
        Error::InvalidConfig(_) => "InvalidConfig",
        Error::InvalidArgument(_) => "InvalidArgument",
        Error::Unsupported(_) => "Unsupported",
        Error::LimitReached(_) => "LimitReached",
        Error::Range(_) => "Range",
        _ => "other",
    }
}

/// One refusal of each kind from each crate: where it comes from, the kind
/// it reports, and the error.
fn refusals() -> Vec<(&'static str, &'static str, Error)> {
    let port = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "Port", BipPortConfig::default()).unwrap(),
    ))
    .unwrap();
    let (_, lease) = db.reserve_bip_port_internal(port, [0; 4], 47808).unwrap();
    let held = db
        .reserve_bip_port_internal(port, [0; 4], 47808)
        .unwrap_err();
    drop(lease);
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let not_a_port = ObjectDatabase::new()
        .reserve_bip_port_internal(device, [0; 4], 47808)
        .unwrap_err();
    let point = AnalogInputObject::new(1, "AI", EngineeringUnits::NO_UNITS).unwrap();
    let no_flags = PreparedFlags {
        value: None,
        encoded: None,
    };
    vec![
        (
            "device-core read/budget.rs",
            "InvalidConfig",
            ReadPropertyMultipleBudget {
                max_result_elements: 0,
                ..Default::default()
            }
            .validate()
            .unwrap_err(),
        ),
        (
            "device-core cov/policy.rs",
            "InvalidConfig",
            CovPolicy {
                max_subscriptions_global: 0,
                ..Default::default()
            }
            .validate()
            .unwrap_err(),
        ),
        (
            "device-core responder.rs",
            "InvalidConfig",
            Responder::new(DeviceIdentity::default(), Limits::default())
                .err()
                .unwrap(),
        ),
        (
            "device-core cov/observation.rs",
            "InvalidArgument",
            validate_flags(&PropertyValue::Boolean(true)).unwrap_err(),
        ),
        (
            "device-core cov/identity.rs",
            "InvalidArgument",
            CovRecipient::Direct(MacAddr::from_slice(&[1; 19]))
                .validate()
                .unwrap_err(),
        ),
        (
            "device-core cov/flags.rs",
            "Unsupported",
            no_flags.selected(&point, None).err().unwrap(),
        ),
        (
            "objects device_identity",
            "InvalidConfig",
            DeviceIdentity::new(1, 1)
                .unwrap()
                .with_max_apdu(10)
                .unwrap_err(),
        ),
        (
            "objects database/network_port.rs",
            "InvalidArgument",
            not_a_port,
        ),
        ("objects database/network_port.rs", "LimitReached", held),
        (
            "objects multistate",
            "Range",
            MultiStateValueObject::new(1, "MSV", 0).err().unwrap(),
        ),
    ]
}

/// Each refusal is the kind its site reports, goes out as the text it
/// replaces did, and stays so when a request decode's reject conversion
/// sees it.
#[test]
fn typed_refusals_go_out_as_the_text_they_replace() {
    let services = [
        ConfirmedServiceChoice::READ_PROPERTY,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
        ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        ConfirmedServiceChoice::ADD_LIST_ELEMENT,
        ConfirmedServiceChoice::CREATE_OBJECT,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
    ];
    let plain = [
        ConfirmedServiceChoice::READ_PROPERTY,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
        ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
    ];
    // `Error` is not `Clone`: the table is built twice, once for the
    // reject conversion.
    for ((site, expected, refusal), (_, _, again)) in refusals().into_iter().zip(refusals()) {
        let as_request = again.into_request_reject();
        for service in services {
            // What the text both kinds replaced went out as (#1833
            // removed it): SERVICES / OTHER in the service's production.
            let text = wire(
                service,
                &Error::protocol(ErrorClass::SERVICES, ErrorCode::OTHER),
            );
            if plain.contains(&service) {
                // Error PDU, invoke ID 9, the service; SERVICES (5) and
                // OTHER (0) as application-tagged ENUMERATED.
                assert_eq!(text, [0x50, 9, service.to_raw(), 0x91, 5, 0x91, 0]);
            }
            assert_eq!(wire(service, &refusal), text, "{site}: {service:?}");
            assert_eq!(wire(service, &as_request), text, "{site}: {service:?}");
        }
        // The bytes first, so a site moved to another kind that still goes
        // out the same way is caught here.
        assert_eq!(kind(&refusal), expected, "{site}: {refusal:?}");
    }
}
