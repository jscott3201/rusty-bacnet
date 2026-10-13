//! The buffer's computed lookups against the walks they replace (#1536):
//! each identity against the numbering the buffer used to derive by
//! stepping back from Total_Record_Count, each sequence lookup against a
//! search of those identities, and the kept timestamp order against one
//! worked out afresh, over random histories of insertions, evictions,
//! clears, restores (#1537), wraps and clock changes.
use super::*;

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

/// The identities as the buffer derived them before #1536: the oldest
/// numbered by stepping back from the count once per newer record.
fn stepped_identities(buffer: &LogRecordBuffer) -> Vec<LogRecordIdentity> {
    let records = buffer.records();
    if records.is_empty() {
        return Vec::new();
    }
    let mut sequence_number = buffer.total_record_count();
    for _ in 1..records.len() {
        sequence_number = if sequence_number == 1 {
            u32::MAX
        } else {
            sequence_number - 1
        };
    }
    records
        .iter()
        .map(|record| {
            let identity =
                LogRecordIdentity::new(u64::from(sequence_number), record.date, record.time)
                    .unwrap();
            sequence_number = next_sequence(sequence_number);
            identity
        })
        .collect()
}

/// The timestamp order worked out from scratch.
fn scanned_order(identities: &[LogRecordIdentity]) -> TimestampOrder {
    let keys: Option<Vec<_>> = identities
        .iter()
        .map(LogRecordIdentity::timestamp_key)
        .collect();
    match keys {
        None => TimestampOrder::Unkeyed,
        Some(keys) if keys.windows(2).any(|pair| pair[1] < pair[0]) => TimestampOrder::Unordered,
        Some(_) => TimestampOrder::Ascending,
    }
}

/// The buffer seen through the trait's required methods only, so its
/// provided lookups run.
struct Walked<'a>(&'a LogRecordBuffer);

impl LogBufferRecords for Walked<'_> {
    fn record_count(&self) -> usize {
        self.0.record_count()
    }

    fn encode_record(&self, index: usize, buf: &mut BytesMut) {
        self.0.encode_record(index, buf);
    }

    fn record_identity(&self, index: usize) -> LogRecordIdentity {
        self.0.record_identity(index)
    }
}

/// A sample stamped `second` seconds into 1 August 2026.
fn stamped(second: u32) -> BACnetLogRecord {
    BACnetLogRecord {
        date: Date {
            year: 126,
            month: 8,
            day: 1 + (second / 86_400 % 28) as u8,
            day_of_week: 1 + (second / 86_400 % 7) as u8,
        },
        time: Time {
            hour: (second / 3_600 % 24) as u8,
            minute: (second / 60 % 60) as u8,
            second: (second % 60) as u8,
            hundredths: 0,
        },
        log_datum: LogDatum::UnsignedValue(u64::from(second)),
        status_flags: None,
    }
}

/// [`stamped`], or, one time in sixteen, with a timestamp that isn't an
/// actual moment.
fn sample(rng: &mut Rng, second: u32) -> BACnetLogRecord {
    let mut record = stamped(second);
    match rng.below(64) {
        0 => record.date.year = Date::UNSPECIFIED,
        1 => record.date.day_of_week = 0,
        2 => record.time.hour = 24,
        3 => record.time.hundredths = Time::UNSPECIFIED,
        _ => {}
    }
    record
}

fn check(buffer: &LogRecordBuffer, rng: &mut Rng, context: &str) {
    let expected = stepped_identities(buffer);
    assert_eq!(buffer.identities(), expected, "{context}");
    for (index, identity) in expected.iter().enumerate() {
        assert_eq!(buffer.record_identity(index), *identity, "{context}");
    }

    let mut probes = vec![0, u64::from(u32::MAX), u64::from(u32::MAX) + 1, u64::MAX];
    probes.extend(expected.iter().map(LogRecordIdentity::sequence_number));
    if let (Some(first), Some(last)) = (expected.first(), expected.last()) {
        for delta in 1..=3 {
            probes.push(first.sequence_number().wrapping_sub(delta));
            probes.push(last.sequence_number().wrapping_add(delta));
        }
    }
    probes.push(u64::from(buffer.total_record_count()));
    probes.push(rng.next());
    probes.push(rng.below(u64::from(u32::MAX) + 2));
    for probe in probes {
        let searched = expected
            .iter()
            .position(|identity| identity.sequence_number() == probe);
        assert_eq!(
            buffer.record_position(probe),
            searched,
            "{context} #{probe}"
        );
        assert_eq!(
            Walked(buffer).record_position(probe),
            searched,
            "{context} #{probe}"
        );
    }

    let order = scanned_order(&expected);
    assert_eq!(buffer.timestamp_order(), order, "{context}");
    assert_eq!(Walked(buffer).timestamp_order(), order, "{context}");
}

#[test]
fn computed_lookups_match_the_walks_over_random_histories() {
    for seed in 0..64 {
        let mut rng = Rng(seed);
        let capacity = [0, 1, 2, 3, 7, 16, 40][rng.below(7) as usize];
        let mut buffer = LogRecordBuffer::new(capacity);
        // Most histories start near the wrap, so they cross it.
        if rng.below(4) != 0 {
            let before_wrap = rng.below(60) as u32;
            buffer
                .restore(u32::MAX - before_wrap, VecDeque::new(), false)
                .unwrap();
        }
        let mut clock = 10_000u32;
        for step in 0..300 {
            let context = format!("seed {seed} step {step} capacity {capacity}");
            match rng.below(40) {
                0 => buffer.clear(),
                1 => {
                    buffer.insert_forced(sample(&mut rng, clock));
                }
                2 => {
                    // A restore (#1537), its count often below its records:
                    // they then number back across the wrap.
                    let held = rng.below(u64::from(capacity) + 1) as u32;
                    let records = (0..held)
                        .map(|offset| sample(&mut rng, clock + 7 * offset))
                        .collect();
                    let total =
                        [1 + rng.below(8) as u32, rng.next() as u32 | 1][rng.below(2) as usize];
                    buffer.restore(total, records, false).unwrap();
                    clock += 7 * held;
                }
                _ => {
                    // Mostly forward, sometimes a repeat, sometimes the clock
                    // set back.
                    clock = match rng.below(12) {
                        0 => clock.saturating_sub(rng.below(5_000) as u32),
                        1 => clock,
                        _ => clock + 1 + rng.below(90) as u32,
                    };
                    buffer.admit_ordinary(sample(&mut rng, clock), true, false);
                }
            }
            check(&buffer, &mut rng, &context);
        }
    }
}

#[test]
fn an_unkeyed_record_outranks_disorder_until_it_is_evicted() {
    let mut buffer = LogRecordBuffer::new(3);
    let mut rng = Rng(7);
    let mut unkeyed = stamped(40);
    unkeyed.date.year = Date::UNSPECIFIED;
    buffer.insert_forced(stamped(50));
    buffer.insert_forced(unkeyed);
    buffer.insert_forced(stamped(30));
    assert_eq!(buffer.timestamp_order(), TimestampOrder::Unkeyed);
    check(&buffer, &mut rng, "unkeyed middle");

    // 50 and 30 are never neighbours, so no step back is counted between
    // them: once the unkeyed record leaves, the buffer runs in order.
    buffer.insert_forced(stamped(60));
    assert_eq!(buffer.timestamp_order(), TimestampOrder::Unkeyed);
    buffer.insert_forced(stamped(70));
    assert_eq!(buffer.timestamp_order(), TimestampOrder::Ascending);
    buffer.insert_forced(stamped(65));
    assert_eq!(buffer.timestamp_order(), TimestampOrder::Unordered);
    check(&buffer, &mut rng, "descent after eviction");
    buffer.clear();
    assert_eq!(buffer.timestamp_order(), TimestampOrder::Ascending);
}
