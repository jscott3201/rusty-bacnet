use super::*;
use crate::handlers::{WriteCommitObserver, WriteTarget};
use bacnet_types::enums::EngineeringUnits;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

type Events = Arc<Mutex<Vec<&'static str>>>;

// Forward effective metadata and actual writes, while observing source dispatch.
struct SourcedObject {
    inner: Box<dyn BACnetObject>,
    events: Events,
}

impl BACnetObject for SourcedObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.inner.read_property(property, index)
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        panic!("the fixture supplies a command origin")
    }
    fn write_property_from(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        origin: &bacnet_objects::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        self.events.lock().unwrap().push("source-write");
        self.inner
            .write_property_from(property, index, value, priority, origin)
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }
    fn property_metadata(&self) -> Cow<'_, [bacnet_objects::property_metadata::PropertyMetadata]> {
        self.inner.property_metadata()
    }
    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        self.inner.is_array_property(property)
    }
}

struct Observer(Events);

impl WriteCommitObserver for Observer {
    fn before(&mut self, _: &ObjectDatabase, target: WriteTarget<'_>) {
        assert_eq!(target.property, PropertyIdentifier::DESCRIPTION);
        self.0.lock().unwrap().push("before");
    }
    fn commit_policy(
        &mut self,
        _: &mut ObjectDatabase,
        _: WriteTarget<'_>,
        _: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        self.0.lock().unwrap().push("policy");
        None
    }
    fn committed(&mut self, _: &mut ObjectDatabase) {
        self.0.lock().unwrap().push("committed");
    }
    fn failed(&mut self, _: &mut ObjectDatabase, _: &Error) {
        self.0.lock().unwrap().push("failed");
    }
}

#[test]
fn indexed_absence_keeps_wpm_prefix_coordinate_and_skips_authorizer_observer_source_suffix() {
    for malformed in [false, true] {
        let objects: Vec<(Box<dyn BACnetObject>, PropertyIdentifier)> = vec![
            (
                Box::new(
                    AnalogInputObject::new(1, "input", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
                ),
                VENDOR,
            ),
            (
                Box::new(NetworkPortObject::new_bip(1, "port", BipPortConfig::default()).unwrap()),
                VENDOR,
            ),
            (Box::new(staging(false)), PropertyIdentifier::STAGE_NAMES),
        ];
        for (inner, absent) in objects {
            let oid = inner.object_identifier();
            let events = Events::default();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(SourcedObject {
                inner,
                events: events.clone(),
            }))
            .unwrap();
            let input = if malformed {
                vec![0x41, 0]
            } else {
                value(PropertyValue::Unsigned(1))
            };
            let mut bytes = BytesMut::new();
            WritePropertyRequest {
                object_identifier: oid,
                property_identifier: absent,
                property_array_index: Some(1),
                property_value: input.clone(),
                priority: None,
            }
            .encode(&mut bytes)
            .unwrap();
            let origin = crate::command_source::test_origin();
            let mut observer = Observer(events.clone());
            assert_error(
                handle_write_property_observed(
                    &mut db,
                    &bytes,
                    Some(&mut observer),
                    None,
                    Some(&origin),
                )
                .map(|_| ()),
                ErrorCode::UNKNOWN_PROPERTY,
            );
            assert!(
                events.lock().unwrap().is_empty(),
                "early WP gate must not reach hooks"
            );

            let prefix = value(PropertyValue::CharacterString("prefix".into()));
            let suffix = value(PropertyValue::CharacterString("unreached".into()));
            bytes.clear();
            WritePropertyMultipleRequest {
                list_of_write_access_specs: vec![WriteAccessSpecification {
                    object_identifier: oid,
                    list_of_properties: vec![
                        BACnetPropertyValue {
                            property_identifier: PropertyIdentifier::DESCRIPTION,
                            property_array_index: None,
                            value: prefix.clone(),
                            priority: None,
                        },
                        BACnetPropertyValue {
                            property_identifier: absent,
                            property_array_index: Some(1),
                            value: input,
                            priority: None,
                        },
                        BACnetPropertyValue {
                            property_identifier: PropertyIdentifier::DESCRIPTION,
                            property_array_index: None,
                            value: suffix,
                            priority: None,
                        },
                    ],
                }],
            }
            .encode(&mut bytes)
            .unwrap();
            let authorize = |attempt: &bacnet_services::wpm::WritePropertyAttempt| {
                assert_eq!(attempt.reference.object_identifier, oid);
                assert_eq!(
                    attempt.reference.property_identifier,
                    PropertyIdentifier::DESCRIPTION.to_raw()
                );
                assert_eq!(attempt.reference.property_array_index, None);
                assert_eq!(attempt.value, prefix);
                events.lock().unwrap().push("authorize");
                Ok(())
            };
            let WritePropertyMultipleOutcome::Error {
                error,
                first_failed_write_attempt,
                committed_oids,
            } = handle_write_property_multiple_observed(
                &mut db,
                &bytes,
                &mut crate::life_safety_cov::LifeSafetyCovSnapshots::default(),
                Some(&authorize),
                Some(&mut observer),
                None,
                Some(&origin),
            )
            else {
                panic!("expected first failed write");
            };
            assert_error(Err(error), ErrorCode::UNKNOWN_PROPERTY);
            assert_eq!(first_failed_write_attempt.object_identifier, oid);
            assert_eq!(
                first_failed_write_attempt.property_identifier,
                absent.to_raw()
            );
            assert_eq!(first_failed_write_attempt.property_array_index, Some(1));
            assert_eq!(committed_oids, vec![oid]);
            assert_eq!(
                db.get(&oid)
                    .unwrap()
                    .read_property(PropertyIdentifier::DESCRIPTION, None)
                    .unwrap(),
                PropertyValue::CharacterString("prefix".into())
            );
            assert_eq!(
                *events.lock().unwrap(),
                vec!["authorize", "before", "policy", "source-write", "committed"]
            );
        }
    }
}
