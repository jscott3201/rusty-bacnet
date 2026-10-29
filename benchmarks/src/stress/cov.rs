//! COV subscription stress scenario.
//!
//! Each step asks for N subscriptions in all, spread over as many subscriber
//! clients as the server's per-subscriber limit needs: subscriber `s` takes
//! Analog Outputs 1 to `k`, so at the server's total several subscribers
//! hold every object and each write fans out to all of them. A writer then
//! changes the objects' Present_Values in turn, and the step counts the
//! notifications each subscriber receives against those it should.
//! Notifications are unconfirmed, or confirmed and acknowledged by each
//! subscriber with `--confirmed`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{make_bip_server_with_db, make_stress_client};
use crate::stress::output::{DegradationPoint, rate};

/// The server's default limit of subscriptions from one subscriber
/// (`CovPolicy::max_subscriptions_per_peer`).
pub const PER_SUBSCRIBER: u64 = 64;

fn make_ao_db(device_instance: u32, count: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(device_instance, 555)
            .expect("valid Device identity")
            .with_name("COV Stress Device"),
        vendor_name: "Rusty BACnet".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();

    for i in 1..=count {
        let ao = AnalogOutputObject::new(i, format!("AO-{}", i), EngineeringUnits::DEGREES_CELSIUS)
            .unwrap();
        db.add(Box::new(ao)).unwrap();
    }
    db
}

/// How many objects each subscriber asks for, to request `total`
/// subscriptions with at most `per_subscriber` from any one of them.
pub fn split(total: u64, per_subscriber: u64) -> Vec<u64> {
    let mut left = total;
    let mut shares = Vec::new();
    while left > 0 {
        let share = left.min(per_subscriber);
        shares.push(share);
        left -= share;
    }
    shares
}

/// Notifications expected from `holders[o]` accepted subscriptions to
/// object `o + 1`, after a write to each object in `written`: one initial
/// notification per subscription, and one per write per subscription.
pub fn expected_notifications(holders: &[u64], written: &[u64]) -> u64 {
    let initial: u64 = holders.iter().sum();
    let per_write: u64 = written
        .iter()
        .zip(holders)
        .map(|(writes, held)| writes * held)
        .sum();
    initial + per_write
}

/// The notifications a step counted: by the time its clock started (the
/// setup notifications), by its end, and in all it should have.
#[derive(Clone, Copy, Debug)]
pub struct Received {
    pub at_start: u64,
    pub total: u64,
    pub expected: u64,
}

/// A step's point: its rate counts only the notifications inside the timed
/// window, while every expected notification that never arrived, setup
/// ones included, counts as an error, with `other_errors`.
pub fn step_point(
    parameter: u64,
    received: Received,
    other_errors: u64,
    elapsed: Duration,
) -> DegradationPoint {
    let timed = received.total.saturating_sub(received.at_start);
    DegradationPoint {
        parameter,
        p50_us: 0, // the step counts notifications; it doesn't time them
        p99_us: 0,
        throughput: rate(timed, elapsed),
        successful: timed,
        errors: other_errors + received.expected.saturating_sub(received.total),
        elapsed_secs: elapsed.as_secs_f64(),
    }
}

pub async fn run(
    duration_secs: u64,
    change_rate_hz: u64,
    confirmed: bool,
    steps: &[u64],
) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();
    let objects = steps.iter().copied().max().unwrap_or(1).min(PER_SUBSCRIBER);

    for &count in steps {
        let shares = split(count, PER_SUBSCRIBER);
        eprintln!(
            "--- {count} COV subscriptions from {} subscribers ({}) ---",
            shares.len(),
            if confirmed {
                "confirmed"
            } else {
                "unconfirmed"
            },
        );

        let db = make_ao_db(1234, objects as u32);
        let mut server = make_bip_server_with_db(db).await.unwrap();
        let server_mac = server.local_mac().to_vec();

        // holders[o]: accepted subscriptions to Analog Output o + 1.
        let mut holders = vec![0u64; objects as usize];
        let received = Arc::new(AtomicU64::new(0));
        let mut subscribers = Vec::new();
        let mut listeners = Vec::new();
        let mut refused = 0u64;
        for &share in &shares {
            let subscriber = make_stress_client().await.unwrap();
            let mut rx = subscriber.cov_notifications();
            let counter = received.clone();
            listeners.push(tokio::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match rx.recv().await {
                        Ok(_) => {
                            counter.fetch_add(1, Ordering::Relaxed);
                        }
                        // Notifications this listener fell behind on are
                        // missing from the count, not an end to it.
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => break,
                    }
                }
            }));
            for i in 1..=share as u32 {
                let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, i).unwrap();
                match subscriber
                    .subscribe_cov(&server_mac, 1, oid, confirmed, Some(600))
                    .await
                {
                    Ok(()) => holders[i as usize - 1] += 1,
                    Err(_) => refused += 1,
                }
            }
            subscribers.push(subscriber);
        }
        let accepted: u64 = holders.iter().sum();
        if refused > 0 {
            eprintln!("  the server refused {refused} subscriptions");
        }

        // Every accepted subscription sends one notification at setup. Wait
        // for those before the clock starts, so the step's rate counts only
        // the notifications its writes caused.
        let setup_until = Instant::now() + Duration::from_secs(2);
        while received.load(Ordering::Relaxed) < accepted && Instant::now() < setup_until {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Write values at change_rate_hz for duration, to the objects in turn.
        let writer = make_stress_client().await.unwrap();
        let interval = 1_000_000u64
            .checked_div(change_rate_hz)
            .map(Duration::from_micros)
            .unwrap_or(Duration::from_secs(1));
        let written_objects = shares.first().copied().unwrap_or(1).max(1);
        let mut written = vec![0u64; objects as usize];
        let at_start = received.load(Ordering::Relaxed);
        let started = Instant::now();
        let deadline = started + Duration::from_secs(duration_secs);
        let mut writes = 0u64;
        let mut write_errors = 0u64;
        let mut value = 100.0f32;
        let mut value_buf = bytes::BytesMut::new();

        while Instant::now() < deadline {
            let instance = writes % written_objects + 1;
            let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, instance as u32).unwrap();
            value += 1.0;
            value_buf.clear();
            bacnet_encoding::primitives::encode_app_real(&mut value_buf, value);
            match writer
                .write_property(
                    &server_mac,
                    oid,
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    value_buf.to_vec(),
                    Some(16),
                )
                .await
            {
                Ok(_) => {
                    writes += 1;
                    written[instance as usize - 1] += 1;
                }
                Err(_) => write_errors += 1,
            }
            tokio::time::sleep(interval).await;
        }

        // Let notifications (and confirmed acknowledgements) drain.
        let expected = expected_notifications(&holders, &written);
        let drain_until = Instant::now() + Duration::from_secs(2);
        while received.load(Ordering::Relaxed) < expected && Instant::now() < drain_until {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let elapsed = started.elapsed();
        let got = received.load(Ordering::Relaxed);

        eprintln!(
            "  accepted={accepted}/{count} writes={writes} write_errors={write_errors} \
             notifications={got}/{expected} expected"
        );

        curve.push(step_point(
            count,
            Received {
                at_start,
                total: got,
                expected,
            },
            write_errors + refused,
            elapsed,
        ));

        drop(writer);
        drop(subscribers);
        for listener in listeners {
            listener.abort();
        }
        let _ = server.stop().await;
    }

    curve
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_spreads_the_total_over_subscribers() {
        assert_eq!(split(1, 64), vec![1]);
        assert_eq!(split(64, 64), vec![64]);
        assert_eq!(split(100, 64), vec![64, 36]);
        assert_eq!(split(1024, 64).len(), 16);
        assert!(split(1024, 64).iter().all(|&s| s == 64));
        assert!(split(0, 64).is_empty());
    }

    #[test]
    fn expected_counts_initial_and_fanned_out_notifications() {
        // Two subscribers hold object 1, one holds object 2; object 1 was
        // written 3 times, object 2 twice.
        assert_eq!(expected_notifications(&[2, 1], &[3, 2]), 3 + 6 + 2);
        assert_eq!(expected_notifications(&[0, 0], &[5, 5]), 0);
    }

    #[test]
    fn the_rate_leaves_out_setup_notifications() {
        // 64 subscriptions (64 setup notifications before the clock), then
        // 20 writes in 2 seconds, each notifying one subscriber.
        let received = Received {
            at_start: 64,
            total: 84,
            expected: 84,
        };
        let pt = step_point(64, received, 0, Duration::from_secs(2));
        assert_eq!(pt.successful, 20);
        assert!((pt.throughput - 10.0).abs() < 1e-9, "{}", pt.throughput);
        assert_eq!(pt.errors, 0);
    }

    #[test]
    fn missing_notifications_count_against_the_total() {
        let received = Received {
            at_start: 64,
            total: 80,
            expected: 84,
        };
        let pt = step_point(64, received, 1, Duration::from_secs(2));
        assert_eq!(pt.successful, 16);
        assert_eq!(pt.errors, 5);
    }
}
