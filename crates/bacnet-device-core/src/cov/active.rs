//! Live Device `Active_COV_Subscriptions` and
//! `Active_COV_Multiple_Subscriptions` projections (Clause 12.11).
//!
//! The [`CovTable`] is the sole live authority for both lists. A read
//! copies the entries live at one sampled instant under the host's table
//! guard, releases that guard, then projects the copy against the caller's
//! database guard. Ordinary and Single entries form
//! `Active_COV_Subscriptions`; Multiple references are never listed there and
//! form `Active_COV_Multiple_Subscriptions` (see
//! [`ActiveCovMultipleSubscriptions`]).
use super::reported::lead;
use super::*;
use crate::read::LiveLists;
use alloc::vec::Vec;
use bacnet_encoding::constructed::encode_cov_subscription_list;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_time::Instant;
use bacnet_types::constructed::{
    BACnetAddress, BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
    BACnetRecipientProcess,
};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use core::cmp::Ordering as CmpOrdering;

mod multiple;
mod overlay;
pub use multiple::{ActiveCovMultipleEntry, ActiveCovMultipleSubscriptions};
pub use overlay::{ActiveCovOverlay, ActiveCovView};

/// One ordinary or Single entry copied from the table at a sampled instant.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveCovEntry {
    endpoint: SubscriberEndpoint,
    process_id: u32,
    object: ObjectIdentifier,
    /// Monitored property and index; `None` for an ordinary whole-object entry.
    property: Option<(PropertyIdentifier, Option<u32>)>,
    confirmed: bool,
    time_remaining: u32,
    explicit_increment: Option<f32>,
}

impl CovTable {
    /// Copy the ordinary and Single entries live at `now`.
    ///
    /// Read-only: an entry whose deadline has been reached is omitted without
    /// purging, so admission and the periodic purge remain the only mutators.
    /// Indefinite entries report zero; finite entries report rounded-up seconds.
    pub fn active_cov_entries(&self, now: Instant) -> Vec<ActiveCovEntry> {
        self.entries()
            .filter_map(|entry| {
                let (process_id, object, property) = match entry.key() {
                    CovSubscriptionKey::Object {
                        process_id, object, ..
                    } => (*process_id, *object, None),
                    CovSubscriptionKey::Property {
                        process_id,
                        object,
                        property,
                        index,
                        ..
                    } => (*process_id, *object, Some((*property, *index))),
                    CovSubscriptionKey::Multiple { .. } => return None,
                };
                let time_remaining = CovTimeRemaining::at(entry.expires_at, now).wire_seconds()?;
                Some(ActiveCovEntry {
                    endpoint: entry.endpoint(),
                    process_id,
                    object,
                    property,
                    confirmed: entry.issue_confirmed_notifications,
                    time_remaining,
                    explicit_increment: entry.cov_increment,
                })
            })
            .collect()
    }
}

/// Request-local value of the local Device's `Active_COV_Subscriptions`.
///
/// Built once per request and reused by every reference to the property in
/// that request, including ReadPropertyMultiple `ALL`/`OPTIONAL` expansion.
#[derive(Debug, Clone)]
pub struct ActiveCovSubscriptions {
    device: ObjectIdentifier,
    encoded: Vec<u8>,
}

impl ActiveCovSubscriptions {
    /// A stopped server services no subscription, so it reports none.
    pub fn stopped(device: ObjectIdentifier) -> Self {
        Self {
            device,
            encoded: Vec::new(),
        }
    }

    /// Project copied entries while the caller holds the database guard.
    /// `entries` come in the table's order, as
    /// [`CovTable::active_cov_entries`] copies them, which is the list's.
    ///
    /// An entry whose monitored object no longer exists has been terminated by
    /// DeleteObject and is omitted, even before table cleanup completes.
    pub fn project(
        db: &ObjectDatabase,
        device: ObjectIdentifier,
        entries: Vec<ActiveCovEntry>,
    ) -> Self {
        // The table's key order is this list's (#2363), so entries come in
        // order and nothing sorts them.
        debug_assert!(
            entries.is_sorted_by(|a, b| order(a, b).is_le()),
            "entries in the table's order"
        );
        let subscriptions: Vec<_> = entries
            .into_iter()
            .filter_map(|entry| {
                let object = db.get(&entry.object)?;
                let (property, index) = entry
                    .property
                    .unwrap_or((lead(entry.object.object_type()).property(), None));
                Some(BACnetCOVSubscription {
                    recipient: BACnetRecipientProcess {
                        recipient: recipient(&entry.endpoint),
                        process_identifier: entry.process_id,
                    },
                    monitored_property_reference: BACnetObjectPropertyReference {
                        object_identifier: entry.object,
                        property_identifier: property.to_raw(),
                        property_array_index: index,
                    },
                    issue_confirmed_notifications: entry.confirmed,
                    time_remaining: entry.time_remaining,
                    cov_increment: increment_in_use(
                        object,
                        property,
                        index,
                        entry.explicit_increment,
                    ),
                })
            })
            .collect();
        let mut encoded = BytesMut::new();
        // Admission holds every recipient to BACnetAddress::MAX_MAC_LEN
        // (`CovRecipient::validate`, #1156), so each entry encodes.
        encode_cov_subscription_list(&mut encoded, &subscriptions)
            .expect("COV admission bounds recipient MACs");
        Self {
            device,
            encoded: encoded.to_vec(),
        }
    }

    /// The live value for exactly the local Device and this property.
    /// `None` leaves every other read to the object.
    pub fn resolve(
        &self,
        oid: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Option<PropertyValue> {
        (oid == self.device && property == PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS)
            .then(|| PropertyValue::ApplicationData(self.encoded.clone()))
    }
}

/// Selected entries copied under one table read guard at one sampled instant.
#[derive(Debug)]
pub struct LiveCovEntries {
    active: Option<Vec<ActiveCovEntry>>,
    multiple: Option<Vec<ActiveCovMultipleEntry>>,
}

impl CovTable {
    /// Copy only the selected lists' entries live at the one instant `now`.
    pub fn live_cov_entries(&self, selection: LiveLists, now: Instant) -> LiveCovEntries {
        LiveCovEntries {
            active: selection.active_cov.then(|| self.active_cov_entries(now)),
            multiple: selection
                .active_cov_multiple
                .then(|| self.active_cov_multiple_entries(now)),
        }
    }
}

/// Request-local values of the local Device's server-owned lists: the COV
/// lists and, when the request reads it, Device_Address_Binding.
///
/// Built once per request and reused by every reference to any of them in
/// that request, including ReadPropertyMultiple `ALL`/`OPTIONAL` expansion.
#[derive(Debug, Clone, Default)]
pub struct LiveDeviceCov {
    active: Option<ActiveCovSubscriptions>,
    multiple: Option<ActiveCovMultipleSubscriptions>,
    /// The local Device's Device_Address_Binding, as the request sampled
    /// it from the binding table.
    address_bindings: Option<(ObjectIdentifier, PropertyValue)>,
}

impl LiveDeviceCov {
    /// A stopped server services no subscription: every selected list is empty.
    pub fn stopped(selection: LiveLists) -> Self {
        Self {
            active: selection
                .active_cov
                .then(|| ActiveCovSubscriptions::stopped(selection.device)),
            multiple: selection
                .active_cov_multiple
                .then(|| ActiveCovMultipleSubscriptions::stopped(selection.device)),
            address_bindings: None,
        }
    }

    /// Project copied entries while the caller holds the database guard.
    pub fn project(db: &ObjectDatabase, selection: LiveLists, entries: LiveCovEntries) -> Self {
        Self {
            active: entries
                .active
                .map(|entries| ActiveCovSubscriptions::project(db, selection.device, entries)),
            multiple: entries.multiple.map(|entries| {
                ActiveCovMultipleSubscriptions::project(db, selection.device, entries)
            }),
            address_bindings: None,
        }
    }

    /// These lists, with `bindings`, sampled from the binding table, as the
    /// local Device's Device_Address_Binding.
    pub fn with_address_bindings(
        mut self,
        selection: LiveLists,
        bindings: Option<PropertyValue>,
    ) -> Self {
        self.address_bindings = bindings.map(|bindings| (selection.device, bindings));
        self
    }

    /// The live value for exactly the local Device and a selected list.
    /// `None` leaves every other read to the object.
    pub fn resolve(
        &self,
        oid: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Option<PropertyValue> {
        self.active
            .as_ref()
            .and_then(|live| live.resolve(oid, property))
            .or_else(|| {
                self.multiple
                    .as_ref()
                    .and_then(|live| live.resolve(oid, property))
            })
            .or_else(|| match &self.address_bindings {
                Some((device, bindings))
                    if *device == oid && property == PropertyIdentifier::DEVICE_ADDRESS_BINDING =>
                {
                    Some(bindings.clone())
                }
                _ => None,
            })
    }
}

/// The table identity is unique, so this is a total, deterministic list order.
fn order(a: &ActiveCovEntry, b: &ActiveCovEntry) -> CmpOrdering {
    let coordinates = |entry: &ActiveCovEntry| {
        (
            entry.object.object_type().to_raw(),
            entry.object.instance_number(),
            entry
                .property
                .map(|(property, index)| (property.to_raw(), index)),
        )
    };
    coordinates(a)
        .cmp(&coordinates(b))
        .then_with(|| endpoint_order(&a.endpoint, &b.endpoint))
        .then_with(|| a.process_id.cmp(&b.process_id))
}

/// Recipient endpoint order shared by both Device COV lists: routed source
/// first, then the immediate link MAC.
fn endpoint_order(a: &SubscriberEndpoint, b: &SubscriberEndpoint) -> CmpOrdering {
    let routed = |endpoint: &SubscriberEndpoint| {
        endpoint
            .network
            .as_ref()
            .map(|source| (source.network, source.mac_address.clone()))
    };
    routed(a).cmp(&routed(b)).then_with(|| a.mac.cmp(&b.mac))
}

/// A direct subscriber is a local-network address (network 0 and its source
/// MAC); a routed subscriber is its remote NPDU source, never the router MAC.
fn recipient(endpoint: &SubscriberEndpoint) -> BACnetRecipient {
    BACnetRecipient::Address(match &endpoint.network {
        Some(source) => BACnetAddress {
            network_number: source.network,
            mac_address: source.mac_address.clone(),
        },
        None => BACnetAddress {
            network_number: 0,
            mac_address: endpoint.mac.clone(),
        },
    })
}

/// Clause 12.11 lists the increment in use for a numeric monitored value; the
/// production's `cov-increment` is used only with numeric datatypes. The
/// effective increment is the one notification preparation applies, resolved
/// against the object's current state. An unreadable or non-numeric value
/// cannot be classified as numeric and omits it.
fn increment_in_use(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
    explicit: Option<f32>,
) -> Option<f32> {
    let increment = prepare::effective_increment(object, property, true, explicit)?;
    let value = object.read_property(property, index).ok()?;
    let (_, numeric) = validate_sample(object, property, index, &value).ok()?;
    // The subscription list reports the increment as a REAL, so a wider
    // object increment (a Large Analog Value's Double) is narrowed here.
    numeric.then_some(increment as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cov::CovNotificationKind;
    use bacnet_encoding::npdu::NpduAddress;
    use bacnet_objects::analog::AnalogValueObject;
    use bacnet_objects::binary::BinaryValueObject;
    use bacnet_time::Duration;
    use bacnet_types::MacAddr;
    use bacnet_types::enums::EngineeringUnits;
    use bacnet_types::enums::ObjectType;
    use bacnet_types::error::Error;

    fn av(instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
    }

    fn device() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()
    }

    fn proposal(
        process: u32,
        object: ObjectIdentifier,
        property: Option<(PropertyIdentifier, Option<u32>)>,
        expires_at: Option<Instant>,
    ) -> CovSubscription {
        CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[0x0A, 0, 0, 5, 0xBA, 0xC0]),
            subscriber_network: None,
            subscriber_process_identifier: process,
            monitored_object_identifier: object,
            issue_confirmed_notifications: false,
            expires_at,
            last_notified_observation: None,
            monitored_property: property.map(|(property, _)| property),
            monitored_property_array_index: property.and_then(|(_, index)| index),
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        }
    }

    #[test]
    fn active_cov_entries_read_one_instant_without_purging_or_multiple() {
        let now = Instant::from_secs(1_000);
        let mut table = CovTable::default();
        table
            .subscribe(
                proposal(1, av(1), None, Some(now + Duration::from_millis(1_500))),
                now,
            )
            .unwrap();
        table
            .subscribe(proposal(2, av(1), None, None), now)
            .unwrap();
        table
            .subscribe(
                proposal(
                    3,
                    av(1),
                    Some((PropertyIdentifier::PRESENT_VALUE, None)),
                    Some(now + Duration::from_secs(1)),
                ),
                now,
            )
            .unwrap();
        let mut multiple = proposal(
            4,
            av(1),
            Some((PropertyIdentifier::PRESENT_VALUE, None)),
            Some(now + Duration::from_secs(60)),
        );
        multiple.notification_kind = CovNotificationKind::Multiple;
        let context = multiple.key().unwrap().multiple_context().unwrap().clone();
        let expires_at = multiple.expires_at.unwrap();
        table
            .subscribe_multiple(
                &context,
                &multiple.endpoint(),
                expires_at,
                0,
                None,
                vec![multiple],
                now,
            )
            .unwrap();

        let mut listed = table.active_cov_entries(now);
        listed.sort_by_key(|entry| entry.process_id);
        assert_eq!(
            listed
                .iter()
                .map(|entry| (entry.process_id, entry.time_remaining))
                .collect::<Vec<_>>(),
            vec![(1, 2), (2, 0), (3, 1)],
            "finite rounds up, indefinite is zero, Multiple is excluded"
        );
        // The Single entry's deadline is reached: omitted, but not purged.
        let mut later = table
            .active_cov_entries(now + Duration::from_secs(1))
            .iter()
            .map(|entry| (entry.process_id, entry.time_remaining))
            .collect::<Vec<_>>();
        later.sort_unstable();
        assert_eq!(later, vec![(1, 1), (2, 0)]);
        assert_eq!(table.len(), 4);
    }

    #[test]
    fn active_cov_projection_maps_identity_increment_and_scope() {
        let mut db = ObjectDatabase::new();
        let mut analog =
            AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        analog
            .write_property(
                PropertyIdentifier::COV_INCREMENT,
                None,
                PropertyValue::Real(2.0),
                None,
            )
            .unwrap();
        db.add(Box::new(analog)).unwrap();
        let bv = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();
        db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
            .unwrap();
        let router = SubscriberEndpoint::new(
            &[0x0A, 0, 0, 1, 0xBA, 0xC0],
            Some(&NpduAddress {
                network: 7,
                mac_address: MacAddr::from_slice(&[0x33]),
            }),
        );
        let direct = SubscriberEndpoint::new(&[0x0A, 0, 0, 5, 0xBA, 0xC0], None);
        let entry =
            |endpoint: &SubscriberEndpoint, process_id, object, property, explicit_increment| {
                ActiveCovEntry {
                    endpoint: endpoint.clone(),
                    process_id,
                    object,
                    property,
                    confirmed: process_id % 2 == 0,
                    time_remaining: 30,
                    explicit_increment,
                }
            };
        let entries = vec![
            // Deleted before cleanup: terminated, so omitted.
            entry(&direct, 9, av(2), None, None),
            entry(
                &router,
                2,
                av(1),
                Some((PropertyIdentifier::PRESENT_VALUE, None)),
                Some(0.5),
            ),
            entry(&direct, 3, bv, None, None),
            entry(&direct, 1, av(1), None, None),
            entry(
                &direct,
                4,
                av(1),
                Some((PropertyIdentifier::PRIORITY_ARRAY, Some(0))),
                Some(1.0),
            ),
        ];
        // In the table's order, as `active_cov_entries` copies them.
        let mut entries = entries;
        entries.sort_by(order);
        let projected = ActiveCovSubscriptions::project(&db, device(), entries);

        let expected =
            |process_identifier, recipient, reference, cov_increment| BACnetCOVSubscription {
                recipient: BACnetRecipientProcess {
                    recipient,
                    process_identifier,
                },
                monitored_property_reference: reference,
                issue_confirmed_notifications: process_identifier % 2 == 0,
                time_remaining: 30,
                cov_increment,
            };
        let local = BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&[0x0A, 0, 0, 5, 0xBA, 0xC0]),
        });
        let remote = BACnetRecipient::Address(BACnetAddress {
            network_number: 7,
            mac_address: MacAddr::from_slice(&[0x33]),
        });
        let pv = PropertyIdentifier::PRESENT_VALUE.to_raw();
        let mut encoded = BytesMut::new();
        encode_cov_subscription_list(
            &mut encoded,
            &[
                // Ordinary numeric Present_Value inherits the object's increment.
                expected(
                    1,
                    local.clone(),
                    BACnetObjectPropertyReference::new(av(1), pv),
                    Some(2.0),
                ),
                // An explicit override wins over the object's increment.
                expected(
                    2,
                    remote,
                    BACnetObjectPropertyReference::new(av(1), pv),
                    Some(0.5),
                ),
                // Index zero is the non-numeric array size: no increment.
                expected(
                    4,
                    local.clone(),
                    BACnetObjectPropertyReference::new_indexed(
                        av(1),
                        PropertyIdentifier::PRIORITY_ARRAY.to_raw(),
                        0,
                    ),
                    None,
                ),
                // A binary value has no numeric increment.
                expected(3, local, BACnetObjectPropertyReference::new(bv, pv), None),
            ],
        )
        .unwrap();
        let live = PropertyValue::ApplicationData(encoded.to_vec());
        assert_eq!(
            projected.resolve(device(), PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS),
            Some(live)
        );
        assert_eq!(
            projected.resolve(device(), PropertyIdentifier::OBJECT_NAME),
            None
        );
        let other_device = ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap();
        assert_eq!(
            projected.resolve(other_device, PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS),
            None
        );
        assert_eq!(
            ActiveCovSubscriptions::stopped(device())
                .resolve(device(), PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS),
            Some(PropertyValue::ApplicationData(Vec::new()))
        );
    }

    #[test]
    fn active_cov_subscriber_address_holds_to_the_bacnet_address_bound() {
        // #1156: the list reports each subscriber as a BACnetAddress, so
        // admission refuses a direct MAC or routed source past
        // BACnetAddress::MAX_MAC_LEN and every admitted one encodes.
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
        let longest = BACnetAddress::MAX_MAC_LEN;
        let mut direct = proposal(1, av(1), None, None);
        direct.subscriber_mac = MacAddr::from_slice(&vec![0xA5; longest]);
        let mut routed = proposal(2, av(1), None, None);
        routed.subscriber_network = Some(NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&vec![0xA5; longest]),
        });
        let now = Instant::from_secs(1_000);
        let mut table = CovTable::default();
        table.subscribe(direct.clone(), now).unwrap();
        table.subscribe(routed.clone(), now).unwrap();
        for mut too_long in [direct, routed] {
            too_long.subscriber_process_identifier += 10;
            match &mut too_long.subscriber_network {
                Some(source) => source.mac_address.push(0xA5),
                None => too_long.subscriber_mac.push(0xA5),
            }
            assert!(matches!(
                table.subscribe(too_long, now),
                Err(Error::Encoding(_))
            ));
        }
        assert_eq!(table.len(), 2);

        let entries = table.active_cov_entries(now);
        let projected = ActiveCovSubscriptions::project(&db, device(), entries);
        let Some(PropertyValue::ApplicationData(list)) =
            projected.resolve(device(), PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS)
        else {
            panic!("live list")
        };
        let mut offset = 0;
        let mut networks = Vec::new();
        while offset < list.len() {
            let (subscription, end) =
                bacnet_encoding::constructed::decode_cov_subscription(&list, offset).unwrap();
            let BACnetRecipient::Address(address) = subscription.recipient.recipient else {
                panic!("address recipient")
            };
            assert_eq!(address.mac_address.len(), longest);
            networks.push(address.network_number);
            offset = end;
        }
        networks.sort_unstable();
        assert_eq!(networks, [0, 7]);
    }

    #[test]
    fn active_cov_access_point_whole_object_names_access_event() {
        assert_eq!(
            lead(ObjectType::ACCESS_POINT).property(),
            PropertyIdentifier::ACCESS_EVENT
        );
        assert_eq!(
            lead(ObjectType::ANALOG_INPUT).property(),
            PropertyIdentifier::PRESENT_VALUE
        );
    }
}
