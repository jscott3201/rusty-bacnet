//! The clocks and seeded state the clock-bound WriteProperty proptest runs
//! each catalogue object with (#2262).
//!
//! Every object sits alone in an [`ObjectDatabase`], which binds a Device
//! clock and a monotonic clock to it as a server's does. Both move one step
//! (one second) before each write, and the host's duties between writes run
//! then too: the monotonic pass, and finishing a Command or Channel run the
//! last write queued. Within a write the clocks stand still, so a stamp a
//! write takes is the current step's date and time.
//!
//! The seeds, each the smallest that makes the state show:
//! - a log record in the Trend Log, Trend Log Multiple and Event Log (the
//!   Audit Log gains its records from Enable writes);
//! - a Command with two action lists, each writing Analog Value 1;
//! - a B/IP Network Port in BBMD mode whose tables take changes, with a
//!   multicast group lent and an owner that activates changes, so the
//!   pending rows take writes and Changes_Pending moves;
//! - a raised event (NORMAL to FAULT, committed through the object's
//!   event-reporting capability) on every object that takes the commit,
//!   with Event_Detection_Enable turned on first where it starts off;
//! - two Averaging samples, as the clock-free variant has;
//! - a Present_Value command from [`origin`] at priorities 8 and 16 on every
//!   object that tracks a source per priority, so a Value_Source correction
//!   at those priorities has an owner, and a lighting output is on at 8;
//! - Blink_Warn_Enable and an Egress_Time on the lighting outputs, so a
//!   warn arms an egress.
//!
//! [`extra_candidates`] and [`prerequisite`] are the targeted pass's
//! values for rows that take only particular ones.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bacnet_encoding::constructed::{
    encode_bdt_entry_list, encode_color_command, encode_lighting_command,
};
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::catalogue::representatives;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::command::CommandObject;
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::{EventStateChange, EventTransition, EventTransitionCommit};
use bacnet_objects::network_port::{BipPortConfig, NetworkPortObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bip_port::{BbmdChanges, BbmdTables, BipPortMode, IpMulticast};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetBDTEntry, BACnetColorCommand,
    BACnetEventLogRecord, BACnetFDTEntry, BACnetHostAddress, BACnetHostNPort,
    BACnetLightingCommand, BACnetLogMultipleRecord, BACnetLogRecord, BACnetXyColor, EventLogDatum,
    LogData, LogDatum, LogValue,
};
use bacnet_types::enums::{
    ColorOperation, EventState, LightingOperation, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;

/// The date every step reads: Monday 2 November 2026.
pub const DATE: Date = Date {
    year: 126,
    month: 11,
    day: 2,
    day_of_week: 1,
};

/// The Device clock and the monotonic clock, both counting the same steps
/// of one second from 08:00:00 on [`DATE`].
#[derive(Default)]
pub struct Steps(AtomicU32);

impl Steps {
    /// How many steps have passed.
    pub fn now(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }

    /// The local time at the current step.
    pub fn time(&self) -> Time {
        let seconds = 8 * 3600 + self.now();
        Time {
            hour: (seconds / 3600 % 24) as u8,
            minute: (seconds / 60 % 60) as u8,
            second: (seconds % 60) as u8,
            hundredths: 0,
        }
    }

    /// The monotonic instant at the current step.
    pub fn monotonic(&self) -> Duration {
        Duration::from_secs(u64::from(self.now()))
    }

    fn step(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

impl ClockReader for Steps {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: DATE,
            local_time: self.time(),
            // Local time five hours behind UTC, with daylight saving on,
            // so a stamp taken in UTC rather than local time shows.
            utc_offset: 300,
            daylight_savings_status: true,
        })
    }
}

/// The origin every `write_property_from` carries: a local producer of
/// Device 1.
pub fn origin() -> CommandOrigin {
    CommandOrigin::Local {
        owner_device: oid(ObjectType::DEVICE, 1),
        initiating_object: None,
    }
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).expect("a valid identifier")
}

/// BBMD tables that take every change, so the pending rows are writable.
#[derive(Default)]
struct Tables;

impl BbmdTables for Tables {
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry> {
        Vec::new()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        false
    }
    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry> {
        Vec::new()
    }
    fn check_broadcast_distribution_table(&self, _table: &[BACnetBDTEntry]) -> Result<(), Error> {
        Ok(())
    }
    fn activate(&self, _changes: &BbmdChanges) -> Result<(), Error> {
        Ok(())
    }
    fn can_change(&self) -> bool {
        true
    }
}

/// A multicast group that takes any change.
struct Group;

impl IpMulticast for Group {
    fn group(&self) -> [u8; 4] {
        [239, 255, 0, 1]
    }
    fn check(&self, _group: [u8; 4]) -> Result<(), Error> {
        Ok(())
    }
    fn activate(&self, _group: [u8; 4]) -> Result<(), Error> {
        Ok(())
    }
    fn can_change(&self) -> bool {
        true
    }
}

/// One catalogue object with its clocks bound and its state seeded, alone
/// in a database.
pub struct Fixture {
    database: ObjectDatabase,
    oid: ObjectIdentifier,
    steps: Arc<Steps>,
    /// The Network Port's owner lease, kept for the fixture's life.
    _lease: Option<Arc<()>>,
}

impl Fixture {
    /// The catalogue object at `index` (modulo the catalogue's size), seeded.
    pub fn new(index: usize) -> Self {
        let mut objects = representatives();
        let object = objects.swap_remove(index % objects.len());
        let oid = object.object_identifier();
        let steps = Arc::new(Steps::default());
        let mut database = ObjectDatabase::new();
        let clock: Arc<dyn ClockReader> = steps.clone();
        database.set_clock_reader(Some(clock));
        let monotonic = steps.clone();
        database.set_monotonic_clock(Some(Arc::new(move || monotonic.monotonic())));
        let lease = match oid.object_type() {
            ObjectType::NETWORK_PORT => Some(bbmd_port(&mut database, oid)),
            _ => {
                database
                    .add(seeded(object, &steps))
                    .expect("a lone catalogue object fits a database");
                None
            }
        };
        let mut fixture = Self {
            database,
            oid,
            steps,
            _lease: lease,
        };
        fixture.raise_event();
        fixture.command_from_origin();
        // A warn blinks and arms an egress only with these set.
        for (property, value) in [
            (
                PropertyIdentifier::BLINK_WARN_ENABLE,
                PropertyValue::Boolean(true),
            ),
            (PropertyIdentifier::EGRESS_TIME, PropertyValue::Unsigned(5)),
        ] {
            let _ = fixture.object().write_property(property, None, value, None);
        }
        fixture
    }

    /// The seeded catalogue object of `object_type`.
    pub fn of(object_type: ObjectType) -> Self {
        let index = representatives()
            .iter()
            .position(|object| object.object_identifier().object_type() == object_type)
            .expect("a catalogue type");
        Self::new(index)
    }

    /// Move both clocks `steps` steps, as [`step`](Self::step) does.
    pub fn wait(&mut self, steps: u32) {
        for _ in 0..steps {
            self.step();
        }
    }

    /// The object under test.
    pub fn object(&mut self) -> &mut dyn BACnetObject {
        self.database
            .get_mut(&self.oid)
            .expect("the fixture's object stays in its database")
    }

    /// The local time at the current step.
    pub fn time(&self) -> Time {
        self.steps.time()
    }

    /// Move both clocks one step and do what a host does between writes:
    /// run the monotonic pass, and finish the run the last write queued, as
    /// though every write it made succeeded.
    pub fn step(&mut self) {
        self.steps.step();
        let now = self.steps.monotonic();
        let object = self.object();
        if let Some(timed) = object.timed() {
            timed.advance_monotonic_time(now);
        }
        if let Some(runner) = object.command_runner_mut()
            && let Some(run) = runner.take_command_run()
        {
            runner.complete_command_run(run.generation, Ok(()));
        }
    }

    /// Commit NORMAL to FAULT on an object that reports events, with the
    /// current step's stamp, so Event_Detection_Enable FALSE has an event to
    /// reset.
    fn raise_event(&mut self) {
        // With detection off no event can be raised (Clause 13.2.2.1):
        // the Binary and Multi-state Outputs start that way, so turn it on.
        let detection = PropertyIdentifier::EVENT_DETECTION_ENABLE;
        if matches!(
            self.object().read_property(detection, None),
            Ok(PropertyValue::Boolean(false))
        ) {
            let _ =
                self.object()
                    .write_property(detection, None, PropertyValue::Boolean(true), None);
        }
        let timestamp = BACnetTimeStamp::DateTime {
            date: DATE,
            time: self.time(),
        };
        if let Some(reporting) = self.object().event_reporting_mut() {
            // An object without the commit channel keeps its NORMAL state.
            let _ = reporting.commit_event_transition(EventTransitionCommit {
                change: EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::FAULT,
                },
                coordinate: EventTransition::ToFault,
                ack_required: true,
                timestamp,
                message_text: None,
            });
        }
    }

    /// Command Present_Value from [`origin`] at priorities 8 and 16 on an
    /// object that tracks a source per priority (it has
    /// Value_Source_Array): the value it reads now, except that a lighting
    /// output is turned on at 8, so a warn has a light to warn about.
    fn command_from_origin(&mut self) {
        let object = self.object();
        let sourced = object
            .property_metadata()
            .iter()
            .any(|row| row.property_identifier == PropertyIdentifier::VALUE_SOURCE_ARRAY);
        let Ok(current) = object.read_property(PropertyIdentifier::PRESENT_VALUE, None) else {
            return;
        };
        if !sourced {
            return;
        }
        let on = match type_of_object(object) {
            ObjectType::LIGHTING_OUTPUT => PropertyValue::Real(60.0),
            ObjectType::BINARY_LIGHTING_OUTPUT => PropertyValue::Enumerated(1),
            _ => current.clone(),
        };
        for (priority, value) in [(8, on), (16, current)] {
            let _ = object.write_property_from(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                value,
                Some(priority),
                &origin(),
            );
        }
    }
}

fn type_of_object(object: &dyn BACnetObject) -> ObjectType {
    object.object_identifier().object_type()
}

/// `object` with the seeds its type takes.
fn seeded(object: Box<dyn BACnetObject>, steps: &Steps) -> Box<dyn BACnetObject> {
    let oid = object.object_identifier();
    let mut object = match oid.object_type() {
        ObjectType::AVERAGING => {
            let mut averaging = AveragingObject::new(oid.instance_number(), object.object_name())
                .expect("the catalogue's Averaging instance");
            for sample in [1.0, 3.0] {
                averaging.add_sample(sample).expect("a finite sample");
            }
            Box::new(averaging) as Box<dyn BACnetObject>
        }
        ObjectType::COMMAND => {
            let mut command = CommandObject::new(oid.instance_number(), object.object_name())
                .expect("the catalogue's Command instance");
            let list = |value: f32| BACnetActionList {
                commands: vec![BACnetActionCommand {
                    device_identifier: None,
                    object_identifier: oid_of(ObjectType::ANALOG_VALUE),
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: PropertyValue::Real(value),
                    priority: Some(8),
                    post_delay: None,
                    quit_on_failure: false,
                    write_successful: false,
                }],
            };
            command
                .set_action(vec![list(1.0), list(2.0)])
                .expect("two encodable action lists");
            Box::new(command)
        }
        _ => object,
    };
    let date = DATE;
    let time = steps.time();
    if let Some(log) = object.log_buffer_mut() {
        // Each type takes its own record kind and refuses the others.
        let _ = log.add_trend_record(BACnetLogRecord {
            date,
            time,
            log_datum: LogDatum::RealValue(1.0),
            status_flags: None,
        });
        let _ = log.add_trend_multiple_record(BACnetLogMultipleRecord {
            date,
            time,
            log_data: LogData::Values(vec![LogValue::RealValue(1.0)]),
        });
        let _ = log.add_event_log_record(BACnetEventLogRecord {
            date,
            time,
            log_datum: EventLogDatum::LogStatus(LogStatus::empty()),
        });
    }
    object
}

fn oid_of(object_type: ObjectType) -> ObjectIdentifier {
    oid(object_type, 1)
}

/// Put a B/IP Network Port in `database`, registered in BBMD mode with
/// [`Tables`], lent a multicast [`Group`] and owned by a host that
/// activates changes. The owner's lease comes back.
fn bbmd_port(database: &mut ObjectDatabase, oid: ObjectIdentifier) -> Arc<()> {
    let config = BipPortConfig {
        ip_address: [127, 0, 0, 1],
        subnet_mask: [255, 0, 0, 0],
        ..BipPortConfig::default()
    };
    let port = NetworkPortObject::new_bip(oid.instance_number(), "NP-1", config)
        .expect("a valid B/IP configuration");
    database.add(Box::new(port)).expect("a lone port");
    let (_, lease) = database
        .reserve_bip_port_internal(oid, [127, 0, 0, 1], 47808)
        .expect("the configured address");
    let tables: Arc<dyn BbmdTables> = Arc::new(Tables);
    database
        .publish_bip_port_internal(
            oid,
            [127, 0, 0, 1],
            47808,
            1476,
            BipPortMode::Bbmd {
                tables: Some(tables),
            },
        )
        .expect("the reserved bind");
    database
        .set_network_port_multicast_internal(oid, Some(Arc::new(Group)))
        .expect("the registered port");
    database
        .set_network_port_activation_internal(oid, true)
        .expect("the registered port");
    lease
}

/// One candidate write: array index, value, priority, and whether it goes
/// through `write_property_from`.
pub type Candidate = (Option<u32>, PropertyValue, Option<u8>, bool);

/// Writes the targeted pass tries beyond the current value and its
/// variations, for the rows that take only particular values.
pub fn extra_candidates(object_type: ObjectType, property: PropertyIdentifier) -> Vec<Candidate> {
    use PropertyIdentifier as P;
    let encoded = |encode: &dyn Fn(&mut BytesMut)| {
        let mut buf = BytesMut::new();
        encode(&mut buf);
        PropertyValue::ApplicationData(buf.to_vec())
    };
    let values = match (object_type, property) {
        // A Date_List entry naming today, so Present_Value turns TRUE.
        (ObjectType::CALENDAR, P::DATE_LIST) => {
            vec![PropertyValue::List(vec![PropertyValue::ApplicationData(
                vec![0x0C, DATE.year, DATE.month, DATE.day, DATE.day_of_week],
            )])]
        }
        // Tomorrow (not yet active) and yesterday (expired), which move
        // Credential_Status and Reason_For_Disable.
        (ObjectType::ACCESS_CREDENTIAL, P::ACTIVATION_TIME | P::EXPIRATION_TIME) => {
            [DATE.day + 1, DATE.day - 1]
                .into_iter()
                .map(|day| {
                    PropertyValue::List(vec![
                        PropertyValue::Date(Date {
                            day,
                            day_of_week: if day > DATE.day { 2 } else { 7 },
                            ..DATE
                        }),
                        PropertyValue::Time(Time {
                            hour: 0,
                            minute: 0,
                            second: 0,
                            hundredths: 0,
                        }),
                    ])
                })
                .collect()
        }
        (ObjectType::COLOR, P::COLOR_COMMAND) => {
            let command = BACnetColorCommand {
                target_color: Some(BACnetXyColor { x: 0.3, y: 0.3 }),
                fade_time: Some(1000),
                ..BACnetColorCommand::new(ColorOperation::FADE_TO_COLOR)
            };
            vec![
                encoded(&|buf| encode_color_command(buf, &command)),
                encoded(&|buf| {
                    encode_color_command(buf, &BACnetColorCommand::new(ColorOperation::STOP))
                }),
            ]
        }
        // At the seeded priority 8: a fade, a ramp, a warn that arms an
        // egress and a stop; then a short fade at a new priority, 4.
        (ObjectType::LIGHTING_OUTPUT, P::LIGHTING_COMMAND) => {
            let at = |priority: u8, command: BACnetLightingCommand| BACnetLightingCommand {
                priority: Some(priority),
                ..command
            };
            let fade = |level: f32, fade_time: u32| BACnetLightingCommand {
                target_level: Some(level),
                fade_time: Some(fade_time),
                ..BACnetLightingCommand::new(LightingOperation::FADE_TO)
            };
            let ramp = BACnetLightingCommand {
                target_level: Some(80.0),
                ramp_rate: Some(10.0),
                ..BACnetLightingCommand::new(LightingOperation::RAMP_TO)
            };
            [
                at(8, fade(40.0, 2000)),
                at(8, ramp),
                at(8, BACnetLightingCommand::new(LightingOperation::WARN_OFF)),
                at(8, BACnetLightingCommand::new(LightingOperation::STOP)),
                at(4, fade(20.0, 100)),
            ]
            .iter()
            .map(|command| encoded(&|buf| encode_lighting_command(buf, command)))
            .collect()
        }
        (ObjectType::NETWORK_PORT, P::BBMD_BROADCAST_DISTRIBUTION_TABLE) => {
            let entry = BACnetBDTEntry {
                bbmd_address: BACnetHostNPort {
                    host: BACnetHostAddress::Ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
                    port: 47808,
                },
                broadcast_mask: Some([255; 4]),
            };
            vec![encoded(&|buf| {
                encode_bdt_entry_list(buf, std::slice::from_ref(&entry)).expect("an encodable BDT")
            })]
        }
        // The warn commands, at the seeded priority 8.
        (ObjectType::LIGHTING_OUTPUT, P::PRESENT_VALUE) => {
            [-1.0, -2.0, -3.0].map(PropertyValue::Real).to_vec()
        }
        (ObjectType::BINARY_LIGHTING_OUTPUT, P::PRESENT_VALUE) => {
            [2, 3, 4, 5].map(PropertyValue::Enumerated).to_vec()
        }
        // A window that closed an hour ago, one that opens in an hour, and
        // no limit again.
        (
            ObjectType::TREND_LOG | ObjectType::TREND_LOG_MULTIPLE | ObjectType::EVENT_LOG,
            P::START_TIME | P::STOP_TIME,
        ) => {
            let at = |hour: u8| {
                PropertyValue::List(vec![
                    PropertyValue::Date(DATE),
                    PropertyValue::Time(Time {
                        hour,
                        minute: 0,
                        second: 0,
                        hundredths: 0,
                    }),
                ])
            };
            let unspecified = PropertyValue::List(vec![
                PropertyValue::Date(Date {
                    year: 255,
                    month: 255,
                    day: 255,
                    day_of_week: 255,
                }),
                PropertyValue::Time(Time {
                    hour: 255,
                    minute: 255,
                    second: 255,
                    hundredths: 255,
                }),
            ]);
            vec![at(7), at(9), unspecified]
        }
        _ => Vec::new(),
    };
    // The warns act at the priority in control, the seeded 8.
    let priority = (property == P::PRESENT_VALUE).then_some(8);
    let mut out: Vec<Candidate> = values
        .into_iter()
        .map(|value| (None, value, priority, true))
        .collect();
    out.extend(crate::write_values::crafted(object_type, property));
    // Last, relinquishing both seeded commands moves the published source.
    if property == P::PRESENT_VALUE {
        out.push((None, PropertyValue::Null, Some(8), true));
        out.push((None, PropertyValue::Null, Some(16), true));
    }
    out
}

/// What the targeted pass writes first when a row takes no candidate as
/// seeded: logging off for an Audit Log's Buffer_Size, and Out_Of_Service
/// TRUE, which opens the simulation writes, for everything else.
pub fn prerequisite(
    object_type: ObjectType,
    property: PropertyIdentifier,
) -> (PropertyIdentifier, PropertyValue) {
    use PropertyIdentifier as P;
    match (object_type, property) {
        (ObjectType::AUDIT_LOG, P::BUFFER_SIZE) => (P::ENABLE, PropertyValue::Boolean(false)),
        _ => (P::OUT_OF_SERVICE, PropertyValue::Boolean(true)),
    }
}
