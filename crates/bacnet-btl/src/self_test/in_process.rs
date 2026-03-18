//! In-process self-test server — spins up a BACnetServer on loopback.

use std::net::Ipv4Addr;
use std::sync::Arc;

use tokio::sync::RwLock;

use bacnet_client::client::BACnetClient;
use bacnet_objects::access_control::{
    AccessCredentialObject, AccessDoorObject, AccessPointObject, AccessRightsObject,
    AccessUserObject, AccessZoneObject, CredentialDataInputObject,
};
use bacnet_objects::accumulator::{AccumulatorObject, PulseConverterObject};
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::audit::{AuditLogObject, AuditReporterObject};
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::color::{ColorObject, ColorTemperatureObject};
use bacnet_objects::command::CommandObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::elevator::{ElevatorGroupObject, EscalatorObject, LiftObject};
use bacnet_objects::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::file::FileObject;
use bacnet_objects::forwarder::NotificationForwarderObject;
use bacnet_objects::group::{GlobalGroupObject, GroupObject, StructuredViewObject};
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use bacnet_objects::lighting::{BinaryLightingOutputObject, ChannelObject, LightingOutputObject};
use bacnet_objects::load_control::LoadControlObject;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::network_port::NetworkPortObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::program::ProgramObject;
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_objects::staging::StagingObject;
use bacnet_objects::timer::TimerObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_objects::value_types::*;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::ObjectType;

use crate::engine::context::{ClientHandle, TestContext};
use crate::iut::capabilities::{IutCapabilities, ObjectDetail};
use crate::report::model::TestMode;

const TEST_DEVICE_INSTANCE: u32 = 99999;

/// An in-process BACnet server for self-testing.
pub struct InProcessServer {
    #[allow(dead_code)] // Kept alive for the server's background tasks
    server: BACnetServer<BipTransport>,
    db: Arc<RwLock<ObjectDatabase>>,
    local_mac: Vec<u8>,
    capabilities: IutCapabilities,
}

impl InProcessServer {
    /// Start the self-test server on an ephemeral loopback port.
    pub async fn start() -> Result<Self, bacnet_types::error::Error> {
        let db = Self::build_test_database();
        let capabilities = Self::build_capabilities(&db);

        let server = BACnetServer::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0) // ephemeral
            .broadcast_address(Ipv4Addr::LOCALHOST)
            .database(db)
            .build()
            .await?;

        let local_mac = server.local_mac().to_vec();
        let db = server.database().clone();

        Ok(Self {
            server,
            db,
            local_mac,
            capabilities,
        })
    }

    pub fn database(&self) -> &Arc<RwLock<ObjectDatabase>> {
        &self.db
    }

    pub fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    pub fn capabilities(&self) -> &IutCapabilities {
        &self.capabilities
    }

    /// Build a TestContext for running tests against this server.
    pub async fn build_context(&self) -> Result<TestContext, bacnet_types::error::Error> {
        let client = BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .broadcast_address(Ipv4Addr::LOCALHOST)
            .apdu_timeout_ms(2000)
            .build()
            .await?;

        Ok(TestContext::new(
            ClientHandle::Bip(client),
            self.local_mac.clone().into(),
            self.capabilities.clone(),
            Some(crate::self_test::SelfTestServer::InProcess(
                // We need to pass self here, but we can't move out of &self.
                // Instead, the SelfTestServer variant will hold the DB Arc directly.
                InProcessServerHandle {
                    db: self.db.clone(),
                },
            )),
            TestMode::SelfTestInProcess,
        ))
    }

    /// Build the full BTL test database with all 64 object types.
    ///
    /// This is also used by the `serve` subcommand to create a standalone
    /// BTL-compliant server for Docker/external testing.
    pub fn build_test_database() -> ObjectDatabase {
        let mut db = ObjectDatabase::new();

        // Collect OIDs for the Device's object list
        let mut object_list = Vec::new();

        // Device object
        let mut device = DeviceObject::new(DeviceConfig {
            instance: TEST_DEVICE_INSTANCE,
            name: "BTL Self-Test Device".into(),
            vendor_name: "Rusty BACnet".into(),
            vendor_id: 555,
            ..DeviceConfig::default()
        })
        .unwrap();
        object_list.push(device.object_identifier());

        // AI:1 — present_value=72.5, units=degrees-fahrenheit (62)
        let mut ai = AnalogInputObject::new(1, "Zone Temp", 62).unwrap();
        ai.set_present_value(72.5);
        object_list.push(ai.object_identifier());
        db.add(Box::new(ai)).unwrap();

        // AO:1 — commandable, units=percent (98)
        let ao = AnalogOutputObject::new(1, "Damper Position", 98).unwrap();
        object_list.push(ao.object_identifier());
        db.add(Box::new(ao)).unwrap();

        // AV:1 — commandable, units=no-units (95)
        let av = AnalogValueObject::new(1, "Setpoint", 95).unwrap();
        object_list.push(av.object_identifier());
        db.add(Box::new(av)).unwrap();

        // BI:1
        let bi = BinaryInputObject::new(1, "Occupancy Sensor").unwrap();
        object_list.push(bi.object_identifier());
        db.add(Box::new(bi)).unwrap();

        // BO:1 — commandable
        let bo = BinaryOutputObject::new(1, "Fan Command").unwrap();
        object_list.push(bo.object_identifier());
        db.add(Box::new(bo)).unwrap();

        // BV:1 — commandable
        let bv = BinaryValueObject::new(1, "Enable Flag").unwrap();
        object_list.push(bv.object_identifier());
        db.add(Box::new(bv)).unwrap();

        // MSI:1 — 4 states
        let msi = MultiStateInputObject::new(1, "Operating Mode", 4).unwrap();
        object_list.push(msi.object_identifier());
        db.add(Box::new(msi)).unwrap();

        // MSO:1 — commandable, 4 states
        let mso = MultiStateOutputObject::new(1, "Speed Select", 4).unwrap();
        object_list.push(mso.object_identifier());
        db.add(Box::new(mso)).unwrap();

        // MSV:1 — commandable, 4 states
        let msv = MultiStateValueObject::new(1, "System Mode", 4).unwrap();
        object_list.push(msv.object_identifier());
        db.add(Box::new(msv)).unwrap();

        // ── Phase 4: Infrastructure objects ──────────────────────────────

        let cal = CalendarObject::new(1, "Holiday Calendar").unwrap();
        object_list.push(cal.object_identifier());
        db.add(Box::new(cal)).unwrap();

        let sched = ScheduleObject::new(
            1,
            "Occupancy Schedule",
            bacnet_types::primitives::PropertyValue::Real(72.0),
        )
        .unwrap();
        object_list.push(sched.object_identifier());
        db.add(Box::new(sched)).unwrap();

        let tl = TrendLogObject::new(1, "Zone Temp Log", 100).unwrap();
        object_list.push(tl.object_identifier());
        db.add(Box::new(tl)).unwrap();

        let ee = EventEnrollmentObject::new(1, "High Temp Alarm", 5).unwrap(); // 5 = OUT_OF_RANGE
        object_list.push(ee.object_identifier());
        db.add(Box::new(ee)).unwrap();

        let nc = NotificationClass::new(1, "Critical Alarms").unwrap();
        object_list.push(nc.object_identifier());
        db.add(Box::new(nc)).unwrap();

        let avg = AveragingObject::new(1, "Zone Temp Avg").unwrap();
        object_list.push(avg.object_identifier());
        db.add(Box::new(avg)).unwrap();

        let cmd = CommandObject::new(1, "Emergency Command").unwrap();
        object_list.push(cmd.object_identifier());
        db.add(Box::new(cmd)).unwrap();

        let lp = LoopObject::new(1, "PID Loop", 98).unwrap(); // 98 = percent
        object_list.push(lp.object_identifier());
        db.add(Box::new(lp)).unwrap();

        let grp = GroupObject::new(1, "Zone Group").unwrap();
        object_list.push(grp.object_identifier());
        db.add(Box::new(grp)).unwrap();

        // ── Phase 5: Value types + structured ────────────────────────────

        let iv = IntegerValueObject::new(1, "Integer Val").unwrap();
        object_list.push(iv.object_identifier());
        db.add(Box::new(iv)).unwrap();

        let piv = PositiveIntegerValueObject::new(1, "Pos Int Val").unwrap();
        object_list.push(piv.object_identifier());
        db.add(Box::new(piv)).unwrap();

        let lav = LargeAnalogValueObject::new(1, "Large Analog Val").unwrap();
        object_list.push(lav.object_identifier());
        db.add(Box::new(lav)).unwrap();

        let csv = CharacterStringValueObject::new(1, "String Val").unwrap();
        object_list.push(csv.object_identifier());
        db.add(Box::new(csv)).unwrap();

        let osv = OctetStringValueObject::new(1, "OctetString Val").unwrap();
        object_list.push(osv.object_identifier());
        db.add(Box::new(osv)).unwrap();

        let bsv = BitStringValueObject::new(1, "BitString Val").unwrap();
        object_list.push(bsv.object_identifier());
        db.add(Box::new(bsv)).unwrap();

        let dv = DateValueObject::new(1, "Date Val").unwrap();
        object_list.push(dv.object_identifier());
        db.add(Box::new(dv)).unwrap();

        let tv = TimeValueObject::new(1, "Time Val").unwrap();
        object_list.push(tv.object_identifier());
        db.add(Box::new(tv)).unwrap();

        let dtv = DateTimeValueObject::new(1, "DateTime Val").unwrap();
        object_list.push(dtv.object_identifier());
        db.add(Box::new(dtv)).unwrap();

        let dpv = DatePatternValueObject::new(1, "DatePattern Val").unwrap();
        object_list.push(dpv.object_identifier());
        db.add(Box::new(dpv)).unwrap();

        let tpv = TimePatternValueObject::new(1, "TimePattern Val").unwrap();
        object_list.push(tpv.object_identifier());
        db.add(Box::new(tpv)).unwrap();

        let dtpv = DateTimePatternValueObject::new(1, "DateTimePattern Val").unwrap();
        object_list.push(dtpv.object_identifier());
        db.add(Box::new(dtpv)).unwrap();

        let gg = GlobalGroupObject::new(1, "Global Group").unwrap();
        object_list.push(gg.object_identifier());
        db.add(Box::new(gg)).unwrap();

        let sv = StructuredViewObject::new(1, "Structured View").unwrap();
        object_list.push(sv.object_identifier());
        db.add(Box::new(sv)).unwrap();

        let el = EventLogObject::new(1, "Event Log", 100).unwrap();
        object_list.push(el.object_identifier());
        db.add(Box::new(el)).unwrap();

        let tlm = TrendLogMultipleObject::new(1, "Trend Log Multiple", 100).unwrap();
        object_list.push(tlm.object_identifier());
        db.add(Box::new(tlm)).unwrap();

        // ── Phase 6: Specialty objects ───────────────────────────────────

        let acc = AccumulatorObject::new(1, "Energy Meter", 95).unwrap();
        object_list.push(acc.object_identifier());
        db.add(Box::new(acc)).unwrap();

        let pc = PulseConverterObject::new(1, "Pulse Converter", 95).unwrap();
        object_list.push(pc.object_identifier());
        db.add(Box::new(pc)).unwrap();

        let prog = ProgramObject::new(1, "Control Program").unwrap();
        object_list.push(prog.object_identifier());
        db.add(Box::new(prog)).unwrap();

        let lsp = LifeSafetyPointObject::new(1, "Fire Detector").unwrap();
        object_list.push(lsp.object_identifier());
        db.add(Box::new(lsp)).unwrap();

        let lsz = LifeSafetyZoneObject::new(1, "Fire Zone").unwrap();
        object_list.push(lsz.object_identifier());
        db.add(Box::new(lsz)).unwrap();

        let ad = AccessDoorObject::new(1, "Main Entrance").unwrap();
        object_list.push(ad.object_identifier());
        db.add(Box::new(ad)).unwrap();

        let lc = LoadControlObject::new(1, "HVAC Load Control").unwrap();
        object_list.push(lc.object_identifier());
        db.add(Box::new(lc)).unwrap();

        let ap = AccessPointObject::new(1, "Card Reader").unwrap();
        object_list.push(ap.object_identifier());
        db.add(Box::new(ap)).unwrap();

        let az = AccessZoneObject::new(1, "Lobby Zone").unwrap();
        object_list.push(az.object_identifier());
        db.add(Box::new(az)).unwrap();

        let au = AccessUserObject::new(1, "Admin User").unwrap();
        object_list.push(au.object_identifier());
        db.add(Box::new(au)).unwrap();

        let ar = AccessRightsObject::new(1, "Admin Rights").unwrap();
        object_list.push(ar.object_identifier());
        db.add(Box::new(ar)).unwrap();

        let ac = AccessCredentialObject::new(1, "Badge #1").unwrap();
        object_list.push(ac.object_identifier());
        db.add(Box::new(ac)).unwrap();

        let cdi = CredentialDataInputObject::new(1, "Badge Reader").unwrap();
        object_list.push(cdi.object_identifier());
        db.add(Box::new(cdi)).unwrap();

        let nf = NotificationForwarderObject::new(1, "Alarm Forwarder").unwrap();
        object_list.push(nf.object_identifier());
        db.add(Box::new(nf)).unwrap();

        let ae = AlertEnrollmentObject::new(1, "Alert Enrollment").unwrap();
        object_list.push(ae.object_identifier());
        db.add(Box::new(ae)).unwrap();

        let ch = ChannelObject::new(1, "Lighting Channel", 1).unwrap();
        object_list.push(ch.object_identifier());
        db.add(Box::new(ch)).unwrap();

        // ── Phase 7: Remaining objects ───────────────────────────────────

        let lo = LightingOutputObject::new(1, "Dimmer").unwrap();
        object_list.push(lo.object_identifier());
        db.add(Box::new(lo)).unwrap();

        let blo = BinaryLightingOutputObject::new(1, "On/Off Light").unwrap();
        object_list.push(blo.object_identifier());
        db.add(Box::new(blo)).unwrap();

        let np = NetworkPortObject::new(1, "BIP Port", 5).unwrap(); // 5 = IPV4
        object_list.push(np.object_identifier());
        db.add(Box::new(np)).unwrap();

        let tmr = TimerObject::new(1, "Delay Timer").unwrap();
        object_list.push(tmr.object_identifier());
        db.add(Box::new(tmr)).unwrap();

        let eg = ElevatorGroupObject::new(1, "Elevator Bank A").unwrap();
        object_list.push(eg.object_identifier());
        db.add(Box::new(eg)).unwrap();

        let esc = EscalatorObject::new(1, "Escalator 1").unwrap();
        object_list.push(esc.object_identifier());
        db.add(Box::new(esc)).unwrap();

        let lift = LiftObject::new(1, "Elevator 1", 10).unwrap(); // 10 floors
        object_list.push(lift.object_identifier());
        db.add(Box::new(lift)).unwrap();

        let file = FileObject::new(1, "Config File", "text/plain").unwrap();
        object_list.push(file.object_identifier());
        db.add(Box::new(file)).unwrap();

        let stg = StagingObject::new(1, "Cooling Stages", 4).unwrap();
        object_list.push(stg.object_identifier());
        db.add(Box::new(stg)).unwrap();

        let alog = AuditLogObject::new(1, "Audit Log", 100).unwrap();
        object_list.push(alog.object_identifier());
        db.add(Box::new(alog)).unwrap();

        let arpt = AuditReporterObject::new(1, "Audit Reporter").unwrap();
        object_list.push(arpt.object_identifier());
        db.add(Box::new(arpt)).unwrap();

        // Color (type 63) — CIE 1931 xy coordinates
        let color = ColorObject::new(1, "Room Color").unwrap();
        object_list.push(color.object_identifier());
        db.add(Box::new(color)).unwrap();

        // Color Temperature (type 64) — Kelvin
        let ct = ColorTemperatureObject::new(1, "Room Color Temp").unwrap();
        object_list.push(ct.object_identifier());
        db.add(Box::new(ct)).unwrap();

        // Set device object list with all objects
        device.set_object_list(object_list);
        db.add(Box::new(device)).unwrap();

        db
    }

    /// Build IutCapabilities from the test database.
    pub fn build_capabilities(db: &ObjectDatabase) -> IutCapabilities {
        let mut caps = IutCapabilities {
            device_instance: TEST_DEVICE_INSTANCE,
            vendor_id: 555,
            vendor_name: "Rusty BACnet".into(),
            model_name: "BTL Self-Test".into(),
            firmware_revision: "0.7.0".into(),
            protocol_revision: 24,
            protocol_version: 1,
            segmentation_supported: 3, // NONE for now
            max_apdu_length: 1476,
            max_segments: 0,
            ..Default::default()
        };

        // All standard services supported by our server
        // ReadProperty(12), WriteProperty(15), ReadPropertyMultiple(14),
        // WritePropertyMultiple(16), WhoIs(32+?), IAm, SubscribeCOV(5),
        // DeviceCommunicationControl(17), ReinitializeDevice(20),
        // WhoHas, IHave, ConfirmedCOVNotification(1),
        // UnconfirmedCOVNotification(2), AcknowledgeAlarm(0),
        // GetEventInformation(29), GetAlarmSummary(3),
        // GetEnrollmentSummary(4)
        for svc in [0, 1, 2, 3, 4, 5, 12, 14, 15, 16, 17, 20, 29] {
            caps.services_supported.insert(svc);
        }

        // Populate object types and list from DB
        for oid in db.list_objects() {
            caps.object_list.push(oid);
            caps.object_types.insert(oid.object_type());

            // Build basic detail for each object
            caps.object_details.insert(
                oid,
                ObjectDetail {
                    object_type: oid.object_type(),
                    property_list: Vec::new(), // populated lazily or not needed for selection
                    supports_cov: matches!(
                        oid.object_type(),
                        ObjectType::ANALOG_INPUT
                            | ObjectType::ANALOG_OUTPUT
                            | ObjectType::ANALOG_VALUE
                            | ObjectType::BINARY_INPUT
                            | ObjectType::BINARY_OUTPUT
                            | ObjectType::BINARY_VALUE
                            | ObjectType::MULTI_STATE_INPUT
                            | ObjectType::MULTI_STATE_OUTPUT
                            | ObjectType::MULTI_STATE_VALUE
                    ),
                    supports_intrinsic_reporting: matches!(
                        oid.object_type(),
                        ObjectType::ANALOG_INPUT
                            | ObjectType::ANALOG_VALUE
                            | ObjectType::BINARY_INPUT
                            | ObjectType::BINARY_VALUE
                            | ObjectType::MULTI_STATE_INPUT
                            | ObjectType::MULTI_STATE_VALUE
                    ),
                    commandable: matches!(
                        oid.object_type(),
                        ObjectType::ANALOG_OUTPUT
                            | ObjectType::ANALOG_VALUE
                            | ObjectType::BINARY_OUTPUT
                            | ObjectType::BINARY_VALUE
                            | ObjectType::MULTI_STATE_OUTPUT
                            | ObjectType::MULTI_STATE_VALUE
                    ),
                    out_of_service_writable: oid.object_type() != ObjectType::DEVICE,
                },
            );
        }

        caps
    }
}

/// Lightweight handle to the in-process server's database for SelfTestServer.
pub struct InProcessServerHandle {
    pub db: Arc<RwLock<ObjectDatabase>>,
}
