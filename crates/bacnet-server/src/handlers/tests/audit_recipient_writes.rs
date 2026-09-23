//! Validate one Audit_Notification_Recipient before a custom object's write arm.
//! This fixture does not add the property to the built-in Device object.

use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use std::borrow::Cow;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

const PROPERTY: PropertyIdentifier = PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT;
// Independent Clause 21 vectors: context [0], Device instance 42/43.
const DEVICE: &[u8] = &[0x0c, 0x02, 0x00, 0x00, 0x2a];
const INITIAL: &[u8] = &[0x0c, 0x02, 0x00, 0x00, 0x2b];

struct RecipientObject {
    value: PropertyValue,
    writes: Arc<AtomicUsize>,
}

impl BACnetObject for RecipientObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        oid()
    }

    fn object_name(&self) -> &str {
        "Recipient boundary fixture"
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[PROPERTY])
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        assert_eq!(property, PROPERTY);
        Ok(self.value.clone())
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _: Option<u32>,
        value: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        assert_eq!(property, PROPERTY);
        // Deliberately accept any value: validation must precede this mutation.
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.value = value;
        Ok(())
    }
}

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap()
}

fn fixture() -> (ObjectDatabase, Arc<AtomicUsize>) {
    let writes = Arc::new(AtomicUsize::new(0));
    let mut db = ObjectDatabase::new();
    db.add(Box::new(RecipientObject {
        value: PropertyValue::ApplicationData(INITIAL.to_vec()),
        writes: writes.clone(),
    }))
    .unwrap();
    (db, writes)
}

fn assert_value(db: &ObjectDatabase, bytes: &[u8]) {
    assert_eq!(
        db.get(&oid())
            .unwrap()
            .read_property(PROPERTY, None)
            .unwrap(),
        PropertyValue::ApplicationData(bytes.to_vec())
    );
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(),
        property_identifier: PROPERTY,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response).unwrap();
    assert_eq!(
        ReadPropertyACK::decode(&response).unwrap().property_value,
        bytes
    );
}

fn wp(db: &mut ObjectDatabase, value: &[u8]) -> Result<ObjectIdentifier, Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(),
        property_identifier: PROPERTY,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request);
    handle_write_property(db, &request)
}

fn wpm(db: &mut ObjectDatabase, values: &[&[u8]]) -> WritePropertyMultipleOutcome {
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid(),
            list_of_properties: values
                .iter()
                .map(|value| BACnetPropertyValue {
                    property_identifier: PROPERTY,
                    property_array_index: None,
                    value: value.to_vec(),
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut request);
    handle_write_property_multiple_detailed(
        db,
        &request,
        &mut crate::life_safety_cov::LifeSafetyCovSnapshots::default(),
    )
}

fn valid_values() -> Vec<&'static [u8]> {
    vec![
        DEVICE,
        // Device zero and highest concrete instance are valid identifiers.
        &[0x0c, 0x02, 0x00, 0x00, 0x00],
        &[0x0c, 0x02, 0x3f, 0xff, 0xfe],
        // [1] { network 0, six-octet B/IP MAC }.
        &[
            0x1e, 0x21, 0x00, 0x65, 0x06, 192, 168, 1, 10, 0xba, 0xc0, 0x1f,
        ],
        // Routed address and local/remote/global broadcast forms are grammar,
        // not a promise that the audit delivery owner can resolve/send them.
        &[0x1e, 0x22, 0x12, 0x34, 0x61, 0x05, 0x1f],
        &[0x1e, 0x21, 0x00, 0x60, 0x1f],
        &[0x1e, 0x22, 0x12, 0x34, 0x60, 0x1f],
        &[0x1e, 0x22, 0xff, 0xff, 0x60, 0x1f],
    ]
}

fn invalid_values() -> Vec<Vec<u8>> {
    vec![
        vec![],
        vec![0x00], // NULL is not a Recipient or a disabled sentinel.
        vec![0x11], // Boolean
        vec![0xc4, 0x02, 0x00, 0x00, 0x2a], // application OID, not choice [0]
        vec![0x2c, 0x02, 0x00, 0x00, 0x2a], // unknown choice [2]
        vec![0x0b, 0x02, 0x00, 0x2a], // short Device contents
        vec![0x0c, 0x00, 0x00, 0x00, 0x2a], // non-Device OID
        vec![0x0c, 0x02, 0x3f, 0xff, 0xff], // wildcard Device
        vec![0x1e, 0x21, 0x00, 0x1f], // missing MAC
        vec![0x1e, 0x91, 0x00, 0x60, 0x1f], // wrong network type
        vec![0x1e, 0x23, 0x01, 0x00, 0x00, 0x60, 0x1f], // network > u16
        vec![0x1e, 0x21, 0x00, 0x00, 0x1f], // wrong MAC type
        vec![0x1e, 0x21, 0x00, 0x60, 0x00, 0x1f], // extra Address field
        [DEVICE, DEVICE].concat(),
        [DEVICE, &[0x00]].concat(),
        [valid_values()[3], DEVICE].concat(),
    ]
}

fn assert_invalid_encoding(error: Error) {
    assert!(
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::INVALID_DATA_ENCODING.to_raw() as u32),
        "{error:?}"
    );
}

#[test]
fn wp_accepts_exact_device_and_address_bytes() {
    for value in valid_values() {
        let (mut db, writes) = fixture();
        assert_eq!(wp(&mut db, value).unwrap(), oid());
        assert_value(&db, value);
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn wpm_accepts_exact_device_and_address_bytes() {
    for value in valid_values() {
        let (mut db, writes) = fixture();
        let WritePropertyMultipleOutcome::Success { committed_oids } = wpm(&mut db, &[value])
        else {
            panic!("valid Recipient rejected: {value:02x?}");
        };
        assert_eq!(committed_oids, vec![oid()]);
        assert_value(&db, value);
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn wp_rejects_invalid_recipient_before_mutation() {
    for value in invalid_values() {
        let (mut db, writes) = fixture();
        assert_invalid_encoding(wp(&mut db, &value).unwrap_err());
        assert_value(&db, INITIAL);
        assert_eq!(writes.load(Ordering::SeqCst), 0, "{value:02x?}");
    }
}

#[test]
fn wpm_rejects_invalid_recipient_preserving_only_committed_prefix() {
    for value in invalid_values() {
        for prefix in [false, true] {
            let (mut db, writes) = fixture();
            let values: Vec<&[u8]> = if prefix {
                vec![DEVICE, &value, INITIAL]
            } else {
                vec![&value, DEVICE]
            };
            let WritePropertyMultipleOutcome::Error {
                error,
                first_failed_write_attempt,
                committed_oids,
            } = wpm(&mut db, &values)
            else {
                panic!("invalid Recipient did not fail semantically: {value:02x?}");
            };
            assert_invalid_encoding(error);
            assert_eq!(first_failed_write_attempt.object_identifier, oid());
            assert_eq!(
                first_failed_write_attempt.property_identifier,
                PROPERTY.to_raw()
            );
            assert_eq!(first_failed_write_attempt.property_array_index, None);
            assert_eq!(committed_oids, if prefix { vec![oid()] } else { vec![] });
            assert_value(&db, if prefix { DEVICE } else { INITIAL });
            assert_eq!(
                writes.load(Ordering::SeqCst),
                usize::from(prefix),
                "{value:02x?}"
            );
        }
    }
}
