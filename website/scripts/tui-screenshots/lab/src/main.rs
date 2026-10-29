//! A loopback-only BACnet lab for the docs site's TUI screenshots. `capture.py`
//! beside this crate builds and runs it; see website/README.md.
//!
//! Topology, every socket on 127.0.0.1:
//!
//! ```text
//!  network 1 (B/IP)                                   network 2 (B/IP)
//!  BBMD-A + device 1001 "AHU-1"   <-BDT->  router port 1 | router port 2 (BBMD)
//!     FDs: the TUI, device 1002 "Plant-2"                  FD: device 2001 "VAV-7"
//!
//!  BACnet/SC: hub + device 3001 "SC Meter"
//! ```
//!
//! Loopback has no usable broadcast between different UDP ports, so every
//! broadcast travels through BBMDs and foreign-device registrations.
//!
//! Usage: `tui-lab DIR`. It writes certificates and `lab.env` into DIR, prints
//! `READY` on stdout, and runs until stdin closes or it gets SIGTERM/SIGINT.

use std::net::{Ipv4Addr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
use bacnet_transport::bip::{BipTransport, ForeignDeviceConfig};
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, Issuer, KeyPair};
use rustls::pki_types::PrivatePkcs8KeyDer;
use tokio::io::AsyncReadExt;

type Boxed = Box<dyn BACnetObject>;
type Res<T> = Result<T, Box<dyn std::error::Error>>;

const VENDOR: u16 = 260;
/// A foreign device trusts only a unicast Forwarded-NPDU from its BBMD, and a
/// datagram to the configured broadcast IP counts as broadcast, so foreign
/// devices must not name 127.0.0.1 as their broadcast address.
const LOOPBACK_BROADCAST: Ipv4Addr = Ipv4Addr::new(127, 255, 255, 255);

fn free_port() -> u16 {
    UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|s| s.local_addr())
        .map(|a| a.port())
        .expect("free loopback UDP port")
}

fn bdt_row(port: u16) -> BdtEntry {
    BdtEntry {
        ip: Ipv4Addr::LOCALHOST.octets(),
        port,
        broadcast_mask: [255, 255, 255, 255],
    }
}

fn bbmd(port: u16, peers: &[u16]) -> BipTransport {
    let mut t = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
    let mut bdt = vec![bdt_row(port)];
    bdt.extend(peers.iter().map(|p| bdt_row(*p)));
    t.enable_bbmd(bdt);
    t.enable_foreign_device_registration(ForeignDevicePolicy::default());
    // Loopback carries no broadcast between ports: forward to peers and FDs only.
    t.set_bbmd_alone_on_subnet(true);
    t
}

fn foreign(port: u16, bbmd_port: u16) -> BipTransport {
    let mut t = BipTransport::new(Ipv4Addr::LOCALHOST, port, LOOPBACK_BROADCAST);
    t.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port,
        ttl: 60,
        renewal_interval: Some(Duration::from_secs(20)),
    });
    t
}

fn identity(instance: u32, name: &str) -> DeviceIdentity {
    DeviceIdentity::new(instance, VENDOR)
        .expect("valid identity")
        .with_name(name.to_string())
}

fn database(instance: u32, name: &str, model: &str, objects: Vec<Boxed>) -> ObjectDatabase {
    let mut device = DeviceObject::new(DeviceConfig {
        identity: identity(instance, name),
        vendor_name: "Rusty Lab".into(),
        model_name: model.into(),
        firmware_revision: "1.2.3".into(),
        application_software_version: "4.5".into(),
        ..Default::default()
    })
    .expect("device object");
    let mut list = vec![device.object_identifier()];
    list.extend(objects.iter().map(|o| o.object_identifier()));
    device.set_object_list(list);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    for o in objects {
        db.add(o).unwrap();
    }
    db
}

fn set(object: &mut dyn BACnetObject, property: PropertyIdentifier, value: PropertyValue) {
    object
        .write_property(property, None, value, None)
        .unwrap_or_else(|e| panic!("lab {property}: {e}"));
}

fn operator(instance: u32) -> CommandOrigin {
    CommandOrigin::Local {
        owner_device: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        initiating_object: None,
    }
}

/// Device 1001, "AHU-1": the main lab device.
fn ahu_database(instance: u32) -> ObjectDatabase {
    let mut ai1 =
        AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
    ai1.set_present_value(72.5);
    ai1.set_description("Zone 3 space temperature (sensor out for calibration)");
    set(
        &mut ai1,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    );
    set(
        &mut ai1,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::OPEN_LOOP.to_raw()),
    );
    let mut ai2 =
        AnalogInputObject::new(2, "Outside Air", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
    ai2.set_present_value(51.0);
    ai2.set_description("Outside air temperature");
    // AI:3 is in alarm: high limit 80 degF, present value 85.
    let mut ai3 =
        AnalogInputObject::new(3, "Discharge Temp", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
    ai3.set_present_value(85.0);
    ai3.set_description("Supply air discharge temperature");
    set(
        &mut ai3,
        PropertyIdentifier::HIGH_LIMIT,
        PropertyValue::Real(80.0),
    );
    set(
        &mut ai3,
        PropertyIdentifier::LOW_LIMIT,
        PropertyValue::Real(45.0),
    );
    set(
        &mut ai3,
        PropertyIdentifier::DEADBAND,
        PropertyValue::Real(1.0),
    );
    set(
        &mut ai3,
        PropertyIdentifier::LIMIT_ENABLE,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![0xC0],
        },
    );
    set(
        &mut ai3,
        PropertyIdentifier::EVENT_ENABLE,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0],
        },
    );
    set(
        &mut ai3,
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyValue::Unsigned(1),
    );
    let mut ai4 =
        AnalogInputObject::new(4, "Duct Static", EngineeringUnits::INCHES_OF_WATER).unwrap();
    ai4.set_present_value(1.25);

    let mut ao1 = AnalogOutputObject::new(1, "Damper", EngineeringUnits::PERCENT).unwrap();
    ao1.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(55.0),
        Some(10),
        &operator(instance),
    )
    .expect("command AO:1 at 10");
    ao1.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(40.0),
        Some(14),
        &operator(instance),
    )
    .expect("command AO:1 at 14");
    let mut ao2 = AnalogOutputObject::new(2, "Cooling Valve", EngineeringUnits::PERCENT).unwrap();
    ao2.set_description("Chilled water valve");

    let mut av1 =
        AnalogValueObject::new(1, "Cooling Setpoint", EngineeringUnits::DEGREES_FAHRENHEIT)
            .unwrap();
    av1.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(74.0),
        Some(16),
        &operator(instance),
    )
    .expect("command AV:1");
    let av2 = AnalogValueObject::new(2, "Heating Setpoint", EngineeringUnits::DEGREES_FAHRENHEIT)
        .unwrap();

    let bi1 = BinaryInputObject::new(1, "Fan Status").unwrap();
    let mut bi2 = BinaryInputObject::new(2, "Filter Alarm").unwrap();
    set(
        &mut bi2,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    );
    let mut bo1 = BinaryOutputObject::new(1, "Supply Fan").unwrap();
    bo1.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(8),
        &operator(instance),
    )
    .expect("command BO:1 at 8");
    let bv1 = BinaryValueObject::new(1, "Occupied").unwrap();

    let msi1 = MultiStateInputObject::new(1, "Economizer State", 4).unwrap();
    let mut mso1 = MultiStateOutputObject::new(1, "Fan Speed", 3).unwrap();
    mso1.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(2),
        Some(12),
        &operator(instance),
    )
    .expect("command MSO:1");
    let msv1 = MultiStateValueObject::new(1, "Mode", 3).unwrap();

    let mut tl1 = TrendLogObject::new(1, "Outside Air Log", 500).unwrap();
    tl1.set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap(),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    tl1.set_log_interval(200).unwrap();

    let schedule =
        ScheduleObject::new(1, "Occupancy Schedule", PropertyValue::Enumerated(0)).unwrap();
    let calendar = CalendarObject::new(1, "Holidays").unwrap();

    database(
        instance,
        "AHU-1",
        "Bench-1",
        vec![
            Box::new(ai1),
            Box::new(ai2),
            Box::new(ai3),
            Box::new(ai4),
            Box::new(ao1),
            Box::new(ao2),
            Box::new(av1),
            Box::new(av2),
            Box::new(bi1),
            Box::new(bi2),
            Box::new(bo1),
            Box::new(bv1),
            Box::new(msi1),
            Box::new(mso1),
            Box::new(msv1),
            Box::new(NotificationClass::new(1, "Alarms").unwrap()),
            Box::new(tl1),
            Box::new(schedule),
            Box::new(calendar),
        ],
    )
}

/// Device 1002, "Plant-2": many points, for paging.
fn plant_database(instance: u32) -> ObjectDatabase {
    let objects = (1..=120)
        .map(|n| {
            let mut av = AnalogValueObject::new(
                n,
                format!("CHW Point {n:03}"),
                EngineeringUnits::DEGREES_FAHRENHEIT,
            )
            .unwrap();
            av.write_property_from(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(40.0 + n as f32 / 10.0),
                Some(16),
                &operator(instance),
            )
            .expect("plant AV");
            Box::new(av) as Boxed
        })
        .collect();
    database(instance, "Plant-2", "Chiller Plant", objects)
}

/// Device 2001, "VAV-7", behind the router on network 2.
fn vav_database(instance: u32) -> ObjectDatabase {
    let mut ai =
        AnalogInputObject::new(1, "Space Temp", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
    ai.set_present_value(70.2);
    let mut flow =
        AnalogInputObject::new(2, "Airflow", EngineeringUnits::CUBIC_FEET_PER_MINUTE).unwrap();
    flow.set_present_value(450.0);
    database(
        instance,
        "VAV-7",
        "VAV Box",
        vec![
            Box::new(ai),
            Box::new(flow),
            Box::new(AnalogOutputObject::new(1, "Damper Cmd", EngineeringUnits::PERCENT).unwrap()),
            Box::new(BinaryValueObject::new(1, "Occupied").unwrap()),
        ],
    )
}

/// Device 3001 on BACnet/SC.
fn sc_database(instance: u32) -> ObjectDatabase {
    let mut kw = AnalogInputObject::new(1, "Demand", EngineeringUnits::KILOWATTS).unwrap();
    kw.set_present_value(312.4);
    database(instance, "SC Meter", "Meter-SC", vec![Box::new(kw)])
}

struct Pki {
    ca: rcgen::Certificate,
    params: CertificateParams,
    key: KeyPair,
}

impl Pki {
    fn new(name: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        let key = KeyPair::generate().unwrap();
        let ca = params.self_signed(&key).unwrap();
        Self { ca, params, key }
    }

    fn leaf(
        &self,
        name: &str,
        usage: ExtendedKeyUsagePurpose,
        days: Option<i64>,
    ) -> (rcgen::Certificate, KeyPair) {
        let mut params = CertificateParams::new(vec![name.into()]).unwrap();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        params.extended_key_usages = vec![usage];
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(days.unwrap_or(365));
        let key = KeyPair::generate().unwrap();
        let cert = params
            .signed_by(&key, &Issuer::from_params(&self.params, &self.key))
            .unwrap();
        (cert, key)
    }
}

fn write_private(path: &Path, text: &str) -> Res<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[tokio::main]
async fn main() -> Res<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let dir = PathBuf::from(std::env::args().nth(1).ok_or("usage: tui-lab DIR")?);
    std::fs::create_dir_all(&dir)?;

    let (p_bbmd, p_r1, p_r2, p_vav, p_plant) = (
        free_port(),
        free_port(),
        free_port(),
        free_port(),
        free_port(),
    );

    // Network 1: BBMD-A hosting device 1001.
    let mut ahu = BACnetServer::generic_builder()
        .device(identity(1001, "AHU-1"))
        .transport(bbmd(p_bbmd, &[p_r1]))
        .database(ahu_database(1001))
        .build()
        .await?;

    // Router: network 1 port (a BBMD peered with BBMD-A), network 2 port (a
    // BBMD that takes foreign devices).
    let StartedRouter {
        router, mut apdus, ..
    } = BACnetRouter::start(
        vec![
            RouterPort {
                transport: bbmd(p_r1, &[p_bbmd]),
                network_number: 1,
            },
            RouterPort {
                transport: bbmd(p_r2, &[]),
                network_number: 2,
            },
        ],
        RouterOptions::new(),
    )
    .await?;
    let mut router = router;
    tokio::spawn(async move { while apdus.recv().await.is_some() {} });

    // Network 2: device 2001 as a foreign device of the router's port 2.
    let mut vav = BACnetServer::generic_builder()
        .device(identity(2001, "VAV-7"))
        .transport(foreign(p_vav, p_r2))
        .database(vav_database(2001))
        .build()
        .await?;

    // Network 1 again: device 1002 as a foreign device of BBMD-A.
    let mut plant = BACnetServer::generic_builder()
        .device(identity(1002, "Plant-2"))
        .transport(foreign(p_plant, p_bbmd))
        .database(plant_database(1002))
        .build()
        .await?;

    // BACnet/SC: site PKI, a hub and device 3001.
    let site = Pki::new("Rusty Lab Site CA");
    let other = Pki::new("Some Other CA");
    std::fs::write(dir.join("ca.pem"), site.ca.pem())?;
    std::fs::write(dir.join("other-ca.pem"), other.ca.pem())?;
    for (stem, days) in [("tui", None), ("tui-expiring", Some(10))] {
        let (cert, key) = site.leaf(
            &format!("{stem}.lab"),
            ExtendedKeyUsagePurpose::ClientAuth,
            days,
        );
        std::fs::write(dir.join(format!("{stem}.pem")), cert.pem())?;
        write_private(&dir.join(format!("{stem}.key")), &key.serialize_pem())?;
    }
    let (hub_cert, hub_key) = site.leaf("127.0.0.1", ExtendedKeyUsagePurpose::ServerAuth, None);
    let hub_tls = ScHubTlsConfig::from_der(
        vec![site.ca.der().clone()],
        vec![hub_cert.der().clone()],
        PrivatePkcs8KeyDer::from(hub_key.serialize_der()).into(),
    )?;
    let mut hub = ScHub::start_with_tls_config(
        "127.0.0.1:0",
        hub_tls,
        [2, 0, 0, 0, 0, 9],
        [9; 16],
        ScHubHandshakeTimeouts::default(),
    )
    .await?;
    let hub_addr = hub.local_addr().ok_or("hub has no address")?;
    let hub_url = format!("wss://{hub_addr}");
    let (dev_cert, dev_key) = site.leaf("meter.lab", ExtendedKeyUsagePurpose::ClientAuth, None);
    let node_tls = ScNodeTlsConfig::from_der(
        vec![site.ca.der().clone()],
        vec![dev_cert.der().clone()],
        PrivatePkcs8KeyDer::from(dev_key.serialize_der()).into(),
    )?;
    let ws = TlsWebSocket::connect(&hub_url, node_tls).await?;
    let transport = ScTransport::new(ws, [2, 0, 0, 0, 0x30, 0x01]).with_device_uuid([0x31; 16]);
    let mut meter = BACnetServer::generic_builder()
        .device(identity(3001, "SC Meter"))
        .transport(transport)
        .database(sc_database(3001))
        .build()
        .await?;

    let env = format!(
        "LAB_BBMD_PORT={p_bbmd}\nLAB_ROUTER1_PORT={p_r1}\nLAB_ROUTER2_PORT={p_r2}\n\
         LAB_VAV_PORT={p_vav}\nLAB_PLANT_PORT={p_plant}\nLAB_HUB_URL={hub_url}\n\
         LAB_HUB_PORT={}\n",
        hub_addr.port()
    );
    std::fs::write(dir.join("lab.env"), &env)?;
    print!("{env}");
    println!("READY");

    // Moving values: Outside Air drifts, Discharge Temp hovers in alarm.
    let oa = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
    let dt = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let mut tick = tokio::time::interval(Duration::from_millis(700));
    let mut stdin = tokio::io::stdin();
    let mut buf = [0u8; 64];
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut n = 0f32;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                n += 1.0;
                let _ = ahu.set_present_value_local(&oa, PropertyValue::Real(51.0 + 4.0 * (n / 5.0).sin())).await;
                let _ = ahu.set_present_value_local(&dt, PropertyValue::Real(85.0 + 1.5 * (n / 3.0).cos())).await;
            }
            read = stdin.read(&mut buf) => {
                if matches!(read, Ok(0) | Err(_)) { break; }
            }
            _ = sigterm.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    eprintln!("tui-lab: stopping");
    macro_rules! stop {
        ($name:literal, $fut:expr_2021) => {{
            let t = std::time::Instant::now();
            let r = tokio::time::timeout(Duration::from_secs(2), $fut).await;
            eprintln!(
                "tui-lab: {} stop {:?} in {:?}",
                $name,
                r.is_ok(),
                t.elapsed()
            );
        }};
    }
    stop!("meter", meter.stop());
    stop!("plant", plant.stop());
    stop!("vav", vav.stop());
    stop!("router", router.stop());
    stop!("ahu", ahu.stop());
    stop!("hub", hub.stop());
    // Exit now: the runtime's shutdown would wait for the blocking stdin
    // reader, which never returns while the pipe stays open (SIGTERM path).
    std::process::exit(0)
}
