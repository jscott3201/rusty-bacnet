//! The time the database guard is held for a write that commits a
//! transition to 1, 4 and 32 recipients (#2537): the commit, the build, the
//! Event Log record and the pushes onto the lanes, all under the guard that
//! commits. A measurement, not a check: run it in release with
//! `cargo test --release -p bacnet-server --lib hold_time -- --ignored --nocapture`.

use super::*;
use crate::server::event_notifications::commit_written_transition;

/// Notification Class 0 with `recipients` unicast destinations.
fn class_to(db: &mut ObjectDatabase, recipients: u8) {
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    for at in 0..recipients {
        class
            .add_destination(BACnetDestination {
                recipient: BACnetRecipient::Address(BACnetAddress {
                    network_number: 0,
                    mac_address: MacAddr::from_slice(&[10, 0, 1, at, 0xBA, 0xC0]),
                }),
                ..local_broadcast_destination()
            })
            .unwrap();
    }
    db.remove(&class.object_identifier()).unwrap();
    db.add(Box::new(class)).unwrap();
}

#[tokio::test]
#[ignore = "a timing measurement; run in release"]
async fn hold_time_of_a_transition_to_1_4_and_32_recipients() {
    const SAMPLES: usize = 2_000;
    for recipients in [1u8, 4, 32] {
        let rig = Rig::start(|db| {
            ai_objects(db);
            class_to(db, recipients);
        })
        .await;
        let writer = rig.server.local_writer();
        let delivery = writer.event_delivery();
        let mut held = Vec::with_capacity(SAMPLES);
        for sample in 0..SAMPLES {
            let reliability = if sample % 2 == 0 {
                Reliability::NO_SENSOR
            } else {
                Reliability::NO_FAULT_DETECTED
            };
            let value = PropertyValue::Enumerated(reliability.to_raw());
            let mut settle = Settle::default();
            {
                let mut db = rig.server.database().write().await;
                let object = db.get_mut(&ai1()).unwrap();
                object
                    .write_property(PropertyIdentifier::RELIABILITY, None, value, None)
                    .unwrap();
                let started = std::time::Instant::now();
                assert!(commit_written_transition(
                    || delivery,
                    &mut db,
                    ai1(),
                    &mut settle
                ));
                held.push(started.elapsed());
            }
            // The lanes drain before the next, as they would between writes.
            if let Some(settled) = settle.into_wait() {
                let _ = settled.await;
            }
        }
        held.sort();
        let at = |q: usize| held[(held.len() - 1) * q / 100];
        eprintln!(
            "hold_time {recipients:>2} recipients: one guard (commit, build, log, push) \
             median {:?}, p90 {:?}",
            at(50),
            at(90)
        );
    }
}
