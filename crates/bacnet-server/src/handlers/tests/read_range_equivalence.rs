//! ReadRange By-Sequence and By-Time of a log's buffer, answered without
//! walking it (#1536), against the walk the handler made before: every
//! identity numbered by stepping back from Total_Record_Count, the
//! reference found by a search of them, and the timestamps validated and
//! scanned in resident order. Over random logs of each family, most seeded
//! (#1537) to cross the Unsigned32 wrap, with evictions, purges, repeated
//! timestamps, a clock set back and timestamps that aren't actual moments,
//! both must give the same bytes or the same error for every request. An
//! Audit Log ring whose numbers skip falls back to the search.
use std::collections::VecDeque;
use std::ops::Range;

use super::*;
use bacnet_encoding::constructed::encode_audit_log_record;
use bacnet_objects::audit::{AuditLogQueryPage, AuditLogStorage};
use bacnet_objects::log_buffer::TimestampOrder;
use bacnet_services::audit::BACnetAuditLogQueryParameters;
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
};

/// One resident record's identity: sequence number, date and time.
type Identity = (u64, Date, Time);

/// A small deterministic generator (SplitMix64), so every failure repeats.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// The local date and time `second` seconds into 1 August 2026.
fn moment(second: u32) -> (Date, Time) {
    let day = second / 86_400;
    (
        Date {
            year: 126,
            month: 8,
            day: 1 + (day % 28) as u8,
            day_of_week: 1 + (day % 7) as u8,
        },
        Time {
            hour: (second / 3_600 % 24) as u8,
            minute: (second / 60 % 60) as u8,
            second: (second % 60) as u8,
            hundredths: (second % 7) as u8,
        },
    )
}

/// [`moment`], or, one time in twenty-five, a timestamp that isn't one.
fn stamp(rng: &mut Rng, second: u32) -> (Date, Time) {
    let (mut date, mut time) = moment(second);
    match rng.below(100) {
        0 => date.year = Date::UNSPECIFIED,
        1 => date.day_of_week = Date::UNSPECIFIED,
        2 => date.day = 32,
        3 => time.hundredths = 100,
        _ => {}
    }
    (date, time)
}

struct Clock((Date, Time));

impl ClockReader for Clock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: self.0 .0,
            local_time: self.0 .1,
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

/// An empty log of `family` whose Total_Record_Count is seeded at `total`
/// (#1537).
fn empty_log(family: LogFamily, capacity: u32, total: u32) -> Box<dyn BACnetObject> {
    match family {
        LogFamily::Trend => {
            let mut log = TrendLogObject::new(1, "TL-1", capacity).unwrap();
            log.restore_log_buffer(total, []).unwrap();
            Box::new(log)
        }
        LogFamily::Event => {
            let mut log = EventLogObject::new(1, "EL-1", capacity).unwrap();
            log.restore_log_buffer(total, []).unwrap();
            Box::new(log)
        }
        LogFamily::TrendMultiple => {
            let mut log = TrendLogMultipleObject::new(1, "TLM-1", capacity).unwrap();
            log.restore_log_buffer(total, []).unwrap();
            Box::new(log)
        }
    }
}

fn add(object: &mut dyn BACnetObject, family: LogFamily, (date, time): (Date, Time)) {
    match family {
        LogFamily::Trend => object.add_trend_record(BACnetLogRecord {
            date,
            time,
            ..record(1)
        }),
        LogFamily::Event => object.add_event_log_record(BACnetEventLogRecord {
            date,
            time,
            ..event_record(1)
        }),
        LogFamily::TrendMultiple => object.add_trend_multiple_record(BACnetLogMultipleRecord {
            date,
            time,
            ..multiple_record(1)
        }),
    }
    .unwrap();
}

/// A random log of `family`: some records, mostly a second or more apart,
/// some at the same moment, some after the clock went back, a few purges.
fn random_log(rng: &mut Rng, family: LogFamily) -> Box<dyn BACnetObject> {
    let capacity = [1, 2, 3, 5, 13, 40][rng.below(6) as usize];
    // Most logs start counting just short of the wrap, so they cross it.
    let total = match rng.below(4) {
        0 => 0,
        _ => u32::MAX - rng.below(80) as u32,
    };
    let mut object = empty_log(family, capacity, total);
    let mut clock = 50_000u32;
    for _ in 0..rng.below(120) {
        clock = match rng.below(16) {
            0 => clock.saturating_sub(rng.below(20_000) as u32),
            1 => clock,
            _ => clock + 1 + rng.below(600) as u32,
        };
        if rng.below(40) == 0 {
            let at = stamp(rng, clock);
            object.bind_clock_internal(Some(Arc::new(Clock(at))));
            // A purge needs a clock that reads an actual moment.
            let _ = object.write_property(
                PropertyIdentifier::RECORD_COUNT,
                None,
                PropertyValue::Unsigned(0),
                None,
            );
        } else {
            add(object.as_mut(), family, stamp(rng, clock));
        }
    }
    object
}

/// The resident identities as the handler derived them before #1536:
/// (sequence number, date, time), the oldest numbered by stepping back from
/// Total_Record_Count once per newer record.
fn stepped_identities(object: &dyn BACnetObject) -> Vec<Identity> {
    let timestamps: Vec<_> = object
        .log_record_identities_internal()
        .unwrap()
        .iter()
        .map(|identity| (identity.date(), identity.time()))
        .collect();
    let PropertyValue::Unsigned(total) = object
        .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
        .unwrap()
    else {
        panic!("Total_Record_Count is Unsigned");
    };
    let mut sequence_number = total as u32;
    for _ in 1..timestamps.len() {
        sequence_number = if sequence_number == 1 {
            u32::MAX
        } else {
            sequence_number - 1
        };
    }
    timestamps
        .into_iter()
        .map(|(date, time)| {
            let identity = (u64::from(sequence_number), date, time);
            sequence_number = if sequence_number == u32::MAX {
                1
            } else {
                sequence_number + 1
            };
            identity
        })
        .collect()
}

/// The pre-#1536 timestamp validation, field by field.
fn civil(date: Date, time: Time) -> Option<(u16, u8, u8, u8, u8, u8, u8)> {
    let year = date.actual_year()?;
    if !(1..=12).contains(&date.month)
        || !(1..=31).contains(&date.day)
        || !(1..=7).contains(&date.day_of_week)
        || time.hour > 23
        || time.minute > 59
        || time.second > 59
        || time.hundredths > 99
    {
        return None;
    }
    Some((
        year,
        date.month,
        date.day,
        time.hour,
        time.minute,
        time.second,
        time.hundredths,
    ))
}

/// Append item `index` of `object`'s Log_Buffer to `buf`.
fn encode_item(object: &dyn BACnetObject, index: usize, buf: &mut BytesMut) {
    match object.log_buffer_internal() {
        Some(records) => records.encode_record(index, buf),
        None => {
            let ring = object.audit_log_storage_internal().unwrap();
            encode_audit_log_record(&ring.retained_records()[index].record, buf).unwrap();
        }
    }
}

/// The ACK the handler gave before #1536 for a log holding `identities`,
/// or the error code it refused with.
fn walked_read(
    object: &dyn BACnetObject,
    identities: &[Identity],
    range: &RangeSpec,
) -> Result<Vec<u8>, u32> {
    let len = identities.len();
    let (anchor, count, refusal) = match *range {
        RangeSpec::BySequenceNumber {
            reference_seq,
            count,
        } => (
            identities
                .iter()
                .position(|identity| identity.0 == reference_seq),
            count,
            None,
        ),
        RangeSpec::ByTime {
            reference_time,
            count,
        } => {
            let reference = civil(reference_time.0, reference_time.1);
            let keys: Option<Vec<_>> = identities
                .iter()
                .map(|&(_, date, time)| civil(date, time))
                .collect();
            match (reference, keys) {
                (Some(reference), Some(keys)) => (
                    if count > 0 {
                        keys.iter().position(|key| *key > reference)
                    } else {
                        keys.iter().rposition(|key| *key < reference)
                    },
                    count,
                    None,
                ),
                _ => (None, count, Some(ErrorCode::LIST_ITEM_NOT_TIMESTAMPED)),
            }
        }
        RangeSpec::ByPosition { .. } => unreachable!("only sequence and time reads"),
    };
    if let Some(code) = refusal {
        return Err(code.to_raw() as u32);
    }
    let selection = super::super::super::read_range::select_signed_range(len, anchor, count);
    let window: Range<usize> = selection.range.clone();
    let mut item_data = BytesMut::new();
    for index in window.clone() {
        encode_item(object, index, &mut item_data);
    }
    let ack = ReadRangeAck {
        object_identifier: object.object_identifier(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: selection.result_flags,
        item_count: window.len() as u32,
        item_data: item_data.to_vec(),
        first_sequence_number: (!window.is_empty()).then(|| identities[window.start].0),
    };
    let mut encoded = BytesMut::new();
    ack.encode(&mut encoded);
    Ok(encoded.to_vec())
}

/// The handler's answer, or `None` for a request the wire can't carry (a
/// reference time with an unspecified field).
fn handled_read(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    range: &RangeSpec,
) -> Option<Result<Vec<u8>, u32>> {
    let mut request = BytesMut::new();
    ReadRangeRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(range.clone()),
    }
    .encode(&mut request)
    .ok()?;
    let mut ack = BytesMut::new();
    Some(match handle_read_range(db, &request, &mut ack) {
        Ok(()) => Ok(ack.to_vec()),
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            Err(code)
        }
        Err(other) => panic!("unexpected {other:?}"),
    })
}

/// Counts the request carries as a nonzero INTEGER16.
const COUNTS: [i32; 10] = [1, 2, 5, -1, -2, -5, 100, -100, 32_767, -32_768];

/// Every request worth asking of a log holding `identities`.
fn requests(rng: &mut Rng, identities: &[Identity]) -> Vec<RangeSpec> {
    let mut sequences = vec![0, u64::from(u32::MAX), u64::from(u32::MAX) + 1, rng.next()];
    let mut moments = vec![moment(rng.below(200_000) as u32), stamp(rng, 0)];
    let mut resident: Vec<_> = identities.to_vec();
    if resident.len() > 6 {
        // A few from anywhere, plus both ends.
        let picks: Vec<_> = (0..4)
            .map(|_| resident[rng.below(resident.len() as u64) as usize])
            .collect();
        resident = [resident[0], resident[resident.len() - 1]]
            .into_iter()
            .chain(picks)
            .collect();
    }
    for (sequence_number, date, time) in resident {
        sequences.extend([
            sequence_number,
            sequence_number.wrapping_sub(1),
            sequence_number.wrapping_add(1),
        ]);
        moments.push((date, time));
        let mut later = time;
        later.hundredths = (later.hundredths + 1).min(99);
        moments.push((date, later));
    }
    let mut ranges = Vec::new();
    for count in COUNTS {
        ranges.extend(
            sequences
                .iter()
                .map(|&reference_seq| RangeSpec::BySequenceNumber {
                    reference_seq,
                    count,
                }),
        );
        ranges.extend(moments.iter().map(|&reference_time| RangeSpec::ByTime {
            reference_time,
            count,
        }));
    }
    ranges
}

/// Ask `object`, whose resident identities are `identities`, every request
/// [`requests`] makes up and compare the handler's answers with the walk's;
/// returns how many were compared.
fn assert_same_answers(
    rng: &mut Rng,
    object: Box<dyn BACnetObject>,
    identities: &[Identity],
    label: &str,
) -> usize {
    let oid = object.object_identifier();
    let ranges = requests(rng, identities);
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();
    let object = db.get(&oid).unwrap();
    let mut compared = 0;
    for range in ranges {
        let Some(handled) = handled_read(&db, oid, &range) else {
            continue;
        };
        assert_eq!(
            handled,
            walked_read(object, identities, &range),
            "{label}: {range:?} over {identities:?}"
        );
        compared += 1;
    }
    compared
}

/// A random Audit Log ring numbered on by one from anywhere, often across
/// the Unsigned64 wrap, stamped the way [`random_log`] stamps its records.
fn random_audit_ring(rng: &mut Rng) -> (Box<dyn BACnetObject>, Vec<Identity>) {
    let len = rng.below(60);
    let mut sequence_number = match rng.below(3) {
        0 => 1 + rng.below(1_000),
        1 => u64::MAX - rng.below(40),
        _ => rng.next().max(1),
    };
    let mut clock = 50_000u32;
    let mut records = VecDeque::new();
    let mut identities = Vec::new();
    for _ in 0..len {
        clock = match rng.below(16) {
            0 => clock.saturating_sub(rng.below(20_000) as u32),
            1 => clock,
            _ => clock + 1 + rng.below(600) as u32,
        };
        // An Audit Log record has to encode, so its fields stay in range;
        // an unspecified year or weekday still leaves it unkeyed.
        let (mut date, mut time) = stamp(rng, clock);
        date.day = date.day.min(31);
        time.hundredths = time.hundredths.min(99);
        identities.push((sequence_number, date, time));
        records.push_back(BACnetAuditLogRecordResult {
            sequence_number,
            record: BACnetAuditLogRecord {
                timestamp: (date, time),
                datum: BACnetAuditLogDatum::TimeChange(clock as f32),
            },
        });
        sequence_number = sequence_number.checked_add(1).unwrap_or(1);
    }
    (Box::new(OddAuditRing { records }), identities)
}

#[test]
fn sequence_and_time_reads_match_the_walk_over_random_logs() {
    let mut compared = 0;
    let mut orders = Vec::new();
    let mut straddling = 0;
    for family in [LogFamily::Trend, LogFamily::Event, LogFamily::TrendMultiple] {
        for seed in 0..64 {
            let mut rng = Rng(seed);
            let object = random_log(&mut rng, family);
            let records = object.log_buffer_internal().unwrap();
            orders.push((records.record_count() > 1, records.timestamp_order()));
            let identities = stepped_identities(object.as_ref());
            if identities.windows(2).any(|pair| pair[1].0 < pair[0].0) {
                straddling += 1;
            }
            let label = format!("{family:?} seed {seed}");
            compared += assert_same_answers(&mut rng, object, &identities, &label);
        }
    }
    for seed in 0..48 {
        let mut rng = Rng(seed);
        let (ring, identities) = random_audit_ring(&mut rng);
        let label = format!("Audit Log seed {seed}");
        compared += assert_same_answers(&mut rng, ring, &identities, &label);
    }
    // Most requests reach the handler; only a reference time with an
    // unspecified field can't be sent.
    assert!(compared > 20_000, "{compared}");
    // Logs holding records from both sides of the wrap are read too.
    assert!(straddling >= 8, "{straddling}");
    // Every way a search by time can go is exercised, on logs long enough
    // to bisect.
    for order in [
        TimestampOrder::Ascending,
        TimestampOrder::Unordered,
        TimestampOrder::Unkeyed,
    ] {
        let seen = orders.iter().filter(|&&seen| seen == (true, order)).count();
        assert!(seen >= 8, "{order:?} {seen}");
    }
}

/// An Audit Log ring as the test makes it: numbered by the store's contract
/// for the random rings, or breaking it, skipping numbers or holding a zero.
struct OddAuditRing {
    records: VecDeque<BACnetAuditLogRecordResult>,
}

impl AuditLogStorage for OddAuditRing {
    fn query(
        &self,
        _parameters: &BACnetAuditLogQueryParameters,
        _start_at_sequence_number: Option<u64>,
        _requested_count: u16,
    ) -> AuditLogQueryPage {
        unreachable!("ReadRange never queries")
    }

    fn retained_records(&self) -> &VecDeque<BACnetAuditLogRecordResult> {
        &self.records
    }
}

impl BACnetObject for OddAuditRing {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::AUDIT_LOG, 3).unwrap()
    }

    fn object_name(&self) -> &str {
        "AL-3"
    }

    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        unreachable!("ReadRange reads the ring")
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        unreachable!("ReadRange is read-only")
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[PropertyIdentifier::LOG_BUFFER])
    }

    fn audit_log_storage_internal(&self) -> Option<&dyn AuditLogStorage> {
        Some(self)
    }
}

fn odd_ring(sequences: &[u64]) -> (ObjectDatabase, ObjectIdentifier) {
    let records = sequences
        .iter()
        .enumerate()
        .map(|(hour, &sequence_number)| BACnetAuditLogRecordResult {
            sequence_number,
            record: BACnetAuditLogRecord {
                timestamp: (DATE, time(hour as u8)),
                datum: BACnetAuditLogDatum::TimeChange(hour as f32),
            },
        })
        .collect();
    let ring = OddAuditRing { records };
    let oid = ring.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(ring)).unwrap();
    (db, oid)
}

#[test]
fn an_audit_ring_that_skips_numbers_is_searched_instead() {
    let (db, oid) = odd_ring(&[5, 6, 9, 10]);
    let by_sequence = |reference_seq, count| {
        call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(RangeSpec::BySequenceNumber {
                reference_seq,
                count,
            }),
        )
        .unwrap()
    };
    // 9 would be the fifth record of a ring counting on from 5; it is the
    // third.
    let found = by_sequence(9, 2);
    assert_eq!(found.item_count, 2);
    assert_eq!(found.result_flags, (false, true, false));
    assert_eq!(found.first_sequence_number, Some(9));
    let back = by_sequence(9, -3);
    assert_eq!(back.item_count, 3);
    assert_eq!(back.first_sequence_number, Some(5));
    // 7 would be the third, which holds 9: absent, not that record.
    let absent = by_sequence(7, 1);
    assert_eq!(absent.item_count, 0);
    assert_eq!(absent.first_sequence_number, None);
}

#[test]
fn an_audit_ring_holding_a_zero_still_refuses_what_it_cannot_place() {
    let (db, oid) = odd_ring(&[5, 0, 7]);
    let read = |range| call(&db, oid, PropertyIdentifier::LOG_BUFFER, Some(range));
    // 7 sits where it is computed to be, two on from 5, so it is served
    // without a search: the zero is never looked for.
    let placed = read(RangeSpec::BySequenceNumber {
        reference_seq: 7,
        count: 1,
    })
    .unwrap();
    assert_eq!(placed.item_count, 1);
    assert_eq!(placed.first_sequence_number, Some(7));
    assert_eq!(placed.result_flags, (false, true, false));
    // Nothing computed holds 8, so the search meets the zero.
    let unnumbered = read(RangeSpec::BySequenceNumber {
        reference_seq: 8,
        count: 1,
    })
    .unwrap_err();
    assert!(matches!(
        unnumbered,
        Error::Protocol { code, .. } if code == ErrorCode::LIST_ITEM_NOT_NUMBERED.to_raw() as u32
    ));
    let untimed = read(RangeSpec::ByTime {
        reference_time: (DATE, time(0)),
        count: 1,
    })
    .unwrap_err();
    assert!(matches!(
        untimed,
        Error::Protocol { code, .. }
            if code == ErrorCode::LIST_ITEM_NOT_TIMESTAMPED.to_raw() as u32
    ));
}
