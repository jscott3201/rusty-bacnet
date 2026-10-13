//! FOREIGN and BBMD modes (#939): which properties each mode adds, their
//! values, and that a BBMD's tables are read live rather than copied.

use super::*;
use crate::database::ObjectDatabase;
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use bacnet_encoding::constructed::{decode_bdt_entry_list, decode_fdt_entry_list};
use bacnet_types::bip_port::{BbmdTables, BipPortMode};
use bacnet_types::constructed::{
    BACnetBDTEntry, BACnetFDTEntry, BACnetHostAddress, BACnetHostNPort,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

const BBMD_ROWS: [P; 3] = [
    P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
    P::BBMD_ACCEPT_FD_REGISTRATIONS,
    P::BBMD_FOREIGN_DEVICE_TABLE,
];
const FOREIGN_ROWS: [P; 2] = [P::FD_BBMD_ADDRESS, P::FD_SUBSCRIPTION_LIFETIME];

/// Tables a test changes between reads.
#[derive(Default)]
struct Tables {
    bdt: Mutex<Vec<BACnetBDTEntry>>,
    accepts: Mutex<bool>,
    fdt: Mutex<Vec<BACnetFDTEntry>>,
}

impl BbmdTables for Tables {
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry> {
        self.bdt.lock().unwrap().clone()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        *self.accepts.lock().unwrap()
    }
    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry> {
        self.fdt.lock().unwrap().clone()
    }
}

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

fn host(ip: [u8; 4], port: u16) -> BACnetHostNPort {
    BACnetHostNPort {
        host: BACnetHostAddress::Ip(IpAddr::V4(Ipv4Addr::from(ip))),
        port,
    }
}

fn bdt_row(ip: [u8; 4]) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: host(ip, 0xBAC0),
        broadcast_mask: Some([0xFF; 4]),
    }
}

/// A database holding port 1, registered and published in `mode`, and the
/// lease that keeps it registered.
fn published(mode: BipPortMode) -> (ObjectDatabase, Arc<()>) {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip(
            1,
            "NP",
            BipPortConfig {
                ip_address: [127, 0, 0, 1],
                udp_port: 0,
                ..Default::default()
            },
        )
        .unwrap(),
    ))
    .unwrap();
    let (_, lease) = db
        .reserve_bip_port_internal(oid(), [127, 0, 0, 1], 0)
        .unwrap();
    db.publish_bip_port_internal(oid(), [127, 0, 0, 1], 47808, 1476, mode)
        .unwrap();
    (db, lease)
}

fn read(db: &ObjectDatabase, property: P) -> Result<PropertyValue, Error> {
    db.get(&oid()).unwrap().read_property(property, None)
}

fn assert_code(result: Result<impl std::fmt::Debug, Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(&result, Err(Error::Protocol { class: c, code: e })
            if *c == class.to_raw() as u32 && *e == code.to_raw() as u32),
        "{result:?}"
    );
}

/// Concatenate a BACnetLIST's framed elements back into list bytes.
fn list_bytes(value: PropertyValue) -> Vec<u8> {
    let PropertyValue::List(items) = value else {
        panic!("a BACnetLIST reads as a list: {value:?}");
    };
    items
        .into_iter()
        .flat_map(|item| match item {
            PropertyValue::ApplicationData(bytes) => bytes,
            other => panic!("each entry is framed: {other:?}"),
        })
        .collect()
}

#[test]
fn each_mode_lists_its_own_rows_read_only_and_required() {
    let tables: Arc<dyn BbmdTables> = Arc::new(Tables::default());
    for (mode, ip_mode, present, absent) in [
        (
            BipPortMode::Normal,
            0,
            &[][..],
            &[&BBMD_ROWS[..], &FOREIGN_ROWS[..]],
        ),
        (
            BipPortMode::Foreign {
                bbmd: host([10, 0, 0, 1], 0xBAC0),
                subscription_lifetime: 300,
            },
            1,
            &FOREIGN_ROWS[..],
            &[&BBMD_ROWS[..], &[][..]],
        ),
        (
            BipPortMode::Bbmd {
                tables: Some(Arc::clone(&tables)),
            },
            2,
            &BBMD_ROWS[..],
            &[&FOREIGN_ROWS[..], &[][..]],
        ),
    ] {
        let (mut db, _lease) = published(mode);
        assert_eq!(
            read(&db, P::BACNET_IP_MODE).unwrap(),
            PropertyValue::Enumerated(ip_mode)
        );
        let object = db.get(&oid()).unwrap();
        let list = object.property_list();
        let required = object.required_properties();
        let metadata = object.property_metadata();
        for &property in present {
            assert!(list.contains(&property), "{property} listed");
            assert!(required.contains(&property), "{property} required");
            let row = metadata
                .iter()
                .find(|row| row.property_identifier == property)
                .unwrap();
            assert_eq!(row.conformance, PropertyConformance::RequiredRead);
            assert_eq!(row.write_capability, PropertyWriteCapability::ReadOnly);
            assert!(read(&db, property).is_ok(), "{property} reads");
        }
        for &property in absent.iter().flat_map(|rows| rows.iter()) {
            assert!(!list.contains(&property), "{property} not listed");
            assert_code(
                read(&db, property),
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }
        // The mode's rows refuse writes, as every configured row does; any
        // other mode's rows don't exist.
        let object = db.get_mut(&oid()).unwrap();
        for &property in present {
            assert_code(
                object.write_property(property, None, PropertyValue::Boolean(true), None),
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        for &property in absent.iter().flat_map(|rows| rows.iter()) {
            assert_code(
                object.write_property(property, None, PropertyValue::Boolean(true), None),
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }
    }
}

#[test]
fn foreign_mode_serves_its_bbmd_and_lifetime() {
    let (db, _lease) = published(BipPortMode::Foreign {
        bbmd: host([10, 0, 0, 1], 0xBAC0),
        subscription_lifetime: 300,
    });
    // [0] { ip-address [1] 10.0.0.1 } port [1] 47808
    assert_eq!(
        read(&db, P::FD_BBMD_ADDRESS).unwrap(),
        PropertyValue::ApplicationData(vec![0x0E, 0x1C, 10, 0, 0, 1, 0x0F, 0x1A, 0xBA, 0xC0])
    );
    assert_eq!(
        read(&db, P::FD_SUBSCRIPTION_LIFETIME).unwrap(),
        PropertyValue::Unsigned(300)
    );
}

#[test]
fn bbmd_mode_reads_the_lent_tables_on_every_read() {
    let tables = Arc::new(Tables::default());
    tables.bdt.lock().unwrap().push(bdt_row([127, 0, 0, 1]));
    let (db, _lease) = published(BipPortMode::Bbmd {
        tables: Some(Arc::clone(&tables) as Arc<dyn BbmdTables>),
    });
    assert_eq!(
        decode_bdt_entry_list(&list_bytes(
            read(&db, P::BBMD_BROADCAST_DISTRIBUTION_TABLE).unwrap()
        ))
        .unwrap(),
        [bdt_row([127, 0, 0, 1])]
    );
    assert_eq!(
        read(&db, P::BBMD_ACCEPT_FD_REGISTRATIONS).unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        read(&db, P::BBMD_FOREIGN_DEVICE_TABLE).unwrap(),
        PropertyValue::List(Vec::new())
    );

    // The owner changes its tables; the next reads show it.
    tables.bdt.lock().unwrap().insert(0, bdt_row([10, 0, 0, 2]));
    *tables.accepts.lock().unwrap() = true;
    let registrant = BACnetFDTEntry {
        address: SocketAddr::from(([10, 0, 0, 9], 0xBAC0)),
        time_to_live: 60,
        remaining_time_to_live: 75,
    };
    tables.fdt.lock().unwrap().push(registrant.clone());
    assert_eq!(
        decode_bdt_entry_list(&list_bytes(
            read(&db, P::BBMD_BROADCAST_DISTRIBUTION_TABLE).unwrap()
        ))
        .unwrap(),
        [bdt_row([10, 0, 0, 2]), bdt_row([127, 0, 0, 1])]
    );
    assert_eq!(
        read(&db, P::BBMD_ACCEPT_FD_REGISTRATIONS).unwrap(),
        PropertyValue::Boolean(true)
    );
    assert_eq!(
        decode_fdt_entry_list(&list_bytes(
            read(&db, P::BBMD_FOREIGN_DEVICE_TABLE).unwrap()
        ))
        .unwrap(),
        [registrant]
    );
}

#[test]
fn a_bbmd_without_tables_is_refused_and_a_new_owner_starts_normal() {
    let (mut db, lease) = published(BipPortMode::Foreign {
        bbmd: host([10, 0, 0, 1], 0xBAC0),
        subscription_lifetime: 300,
    });
    drop(lease);
    // The next owner reserves the bound port the first one published: the
    // port reads NORMAL until it publishes.
    let (_, lease) = db
        .reserve_bip_port_internal(oid(), [127, 0, 0, 1], 47808)
        .unwrap();
    assert_eq!(
        read(&db, P::BACNET_IP_MODE).unwrap(),
        PropertyValue::Enumerated(0)
    );
    // A BBMD that has not started has no tables to lend.
    assert!(db
        .publish_bip_port_internal(
            oid(),
            [127, 0, 0, 1],
            47808,
            1476,
            BipPortMode::Bbmd { tables: None },
        )
        .is_err());
    assert_eq!(db.registered_bip_port_internal(), None);
    assert_eq!(
        read(&db, P::BACNET_IP_MODE).unwrap(),
        PropertyValue::Enumerated(0)
    );
    drop(lease);
}
