//! A Channel's members are written side by side, each when its own
//! Execution_Delay is up, so a member in another device that waits for its
//! answer holds back no other member (Clause 12.53.12, #1343). Requests in
//! other devices wait in the server's queues: one at a time per device, and
//! at most 32 across the server (`command_lists::remote_slots`).
//!
//! CH-5 (channel 21) writes AO-1 in Device 9 at once and the local AO-2
//! 100 ms after the distribution starts. CH-10 (channel 26) writes the
//! Present_Values of AO-1 to AO-20 in Device 9, all at once. CH-11 and CH-12
//! (channels 27 and 28) each write AO-1 in twenty other devices, Devices 100
//! to 119 and 120 to 139. CH-14 (channel 30) writes AO-1 in Device 9 and the
//! local AO-2 together, 100 ms after the distribution starts. Every device is
//! bound to the harness peer and answers by hand. The clock is paused.
use super::channel_remote_write_tests::{
    answer_reads, error, next_reads, next_request, read_ack, reliability, remote_output, start_with,
};
use super::channel_wire_tests::{ch, channel, member, settled, write_channel, write_status};
use super::command_action_wire_tests::{ao, read_db, slot8};
use super::command_remote_write_tests::{ack, device, remote_write, sent_writes};
use super::command_runs::CommandRunner;
use super::cov_wire_test_support::*;
use super::*;
use bacnet_types::enums::{Reliability, WriteStatus};
use std::future::Future;

async fn start() -> Harness {
    let h = start_with(|db| {
        let members = (1..=20)
            .map(|output| (remote_output(9, output), 0))
            .collect();
        db.add(Box::new(channel(10, 26, members))).unwrap();
        for (instance, first) in [(11, 100), (12, 120)] {
            let members = (first..first + 20)
                .map(|device| (remote_output(device, 1), 0))
                .collect();
            db.add(Box::new(channel(instance, 16 + instance as u16, members)))
                .unwrap();
        }
        let together = vec![(remote_output(9, 1), 100), (member(ao(2), PV), 100)];
        db.add(Box::new(channel(14, 30, together))).unwrap();
    })
    .await;
    for instance in 100..140 {
        let binding = DeviceBinding::local(device(instance), PEER).unwrap();
        h.server
            .device_bindings
            .write()
            .await
            .insert_configured(binding, |_| false)
            .unwrap();
    }
    h
}

/// Wait for the WriteProperty requests sent next, at least one.
async fn next_writes(
    h: &Harness,
) -> Vec<(u8, bacnet_services::write_property::WritePropertyRequest)> {
    tokio::time::sleep(Duration::from_millis(10)).await;
    let sent = sent_writes(h);
    assert!(!sent.is_empty(), "a WriteProperty to another device");
    sent
}

#[tokio::test(start_paused = true)]
async fn a_silent_member_in_another_device_holds_back_no_member_here() {
    let mut h = start().await;
    let started = tokio::time::Instant::now();
    write_channel(&mut h, 5, &PropertyValue::Real(80.0), Some(8))
        .await
        .unwrap();
    answer_reads(&h, 1, &PropertyValue::Real(0.0)).await;
    // Device 9 never answers the write; AO-2 is written at its 100 ms all
    // the same.
    remote_write(&h).await;
    tokio::time::sleep_until(started + Duration::from_millis(99)).await;
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
    tokio::time::sleep_until(started + Duration::from_millis(101)).await;
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    assert_eq!(write_status(&mut h, 5).await, WriteStatus::IN_PROGRESS);

    // Stopped while the write is outstanding, the run ends FAILED and the
    // write's invoke ID is free.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(h.server.notification_transactions.active_count(), 1);
    h.server.stop().await.unwrap();
    assert_eq!(
        read_db(&h, ch(5), PropertyIdentifier::WRITE_STATUS, None).await,
        PropertyValue::Enumerated(WriteStatus::FAILED.to_raw())
    );
    assert_eq!(
        read_db(&h, ch(5), PropertyIdentifier::RELIABILITY, None).await,
        PropertyValue::Enumerated(Reliability::PROCESS_ERROR.to_raw())
    );
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}

/// AO-1 there and AO-2 here come due together while something else holds the
/// database. Once it lets go, the database lock grants AO-1's waiting read
/// whether or not AO-1's future is polled. A run that left that future
/// unpolled while it wrote AO-2 kept the read guard for good, and AO-2's
/// write, which needs the database to itself, waited on it with every later
/// user of the database queued behind, a client's ReadProperty included.
#[tokio::test(start_paused = true)]
async fn members_due_together_while_the_database_is_held_both_go() {
    let mut h = start().await;
    let started = tokio::time::Instant::now();
    write_channel(&mut h, 14, &PropertyValue::Real(80.0), Some(8))
        .await
        .unwrap();
    answer_reads(&h, 1, &PropertyValue::Real(0.0)).await;
    let writer = h.server.database().write().await;
    tokio::time::sleep_until(started + Duration::from_millis(150)).await;
    drop(writer);

    let (invoke_id, written) = next_request(&h, ao(1)).await;
    assert_eq!(written, [0x44, 0x42, 0xA0, 0x00, 0x00]);
    h.respond(ack(invoke_id)).await;
    assert_eq!(settled(&mut h, 14).await, WriteStatus::SUCCESSFUL);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
}

/// The same with a waiting writer. AO-1's datatype read is answered while a
/// reader holds the database and before AO-2's 100 ms, so keeping that
/// datatype waits for the write guard and AO-2 comes due behind it. Once the
/// reader lets go the guard goes to AO-1's future, which the run polls while
/// it writes AO-2, so AO-2 is written without waiting for AO-1's answer. A
/// loop that awaited AO-2's write on its own left that guard with an
/// unpolled future and the database locked for good (#1343). The checks on
/// when the read goes out keep a run that wrote the members here first,
/// which never forms that state, from passing (#2514).
#[tokio::test(start_paused = true)]
async fn a_member_here_due_while_one_there_waits_for_the_database_goes_through() {
    let mut h = start().await;
    let started = tokio::time::Instant::now();
    write_channel(&mut h, 5, &PropertyValue::Real(80.0), Some(8))
        .await
        .unwrap();
    let [(invoke_id, request)]: [_; 1] = next_reads(&h, 1).await.try_into().unwrap();
    // AO-1's read goes out when the distribution starts, AO-2 still unwritten.
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
    // A reader, such as a client's ReadProperty, holds the database when the
    // datatype read is answered, so keeping the datatype waits for it.
    let reader = h.server.database().read().await;
    let requests = crate::command_lists::remember_hook::requests();
    h.respond(read_ack(invoke_id, &request, &PropertyValue::Real(0.0)))
        .await;
    // The run's own write for the datatype is the one to wait for: the
    // server's tasks queue for the guard too, so a queued writer alone proves
    // nothing. Its ask is counted in the poll that first queues it.
    tokio::time::timeout(Duration::from_millis(50), async {
        while crate::command_lists::remember_hook::requests() == requests {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the run waits for the database to keep AO-1's datatype");
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "AO-1 waits for the database before AO-2 is due"
    );
    // AO-2's 100 ms pass while the reader still holds it.
    tokio::time::sleep(Duration::from_millis(150)).await;
    drop(reader);

    // AO-2 goes through once the reader lets go, with AO-1's write unanswered.
    tokio::time::timeout(Duration::from_millis(50), async {
        while slot8(&h, ao(2)).await != PropertyValue::Real(80.0) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("AO-2 is written once the database is free");
    let (invoke_id, written) = next_request(&h, ao(1)).await;
    assert_eq!(written, [0x44, 0x42, 0xA0, 0x00, 0x00]);
    h.respond(ack(invoke_id)).await;
    assert_eq!(settled(&mut h, 5).await, WriteStatus::SUCCESSFUL);
}

#[tokio::test(start_paused = true)]
async fn a_distribution_dropped_mid_write_frees_its_invoke_id_without_stop() {
    let h = start().await;
    // CH-5's run, made by hand: NULL needs no read, so AO-1's write is the
    // first request.
    let run = {
        let mut db = h.server.database().write().await;
        let channel = db.get_mut(&ch(5)).unwrap();
        channel
            .write_property(PV, None, PropertyValue::Null, Some(8))
            .unwrap();
        let runs = channel.command_runner_mut().unwrap();
        runs.take_command_run().unwrap()
    };
    let runner = CommandRunner::for_server(&h.server);
    let mut running = Box::pin(crate::command_lists::execute(&runner, run));
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    for _ in 0..100 {
        assert!(running.as_mut().poll(&mut cx).is_pending());
        if h.server.notification_transactions.active_count() == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(h.server.notification_transactions.active_count(), 1);

    // Dropping the run, with the server still running, frees the write's
    // invoke ID itself and ends the run FAILED.
    drop(running);
    assert!(!h.server.notification_transactions.is_closed());
    assert_eq!(h.server.notification_transactions.active_count(), 0);
    assert_eq!(
        h.server.notification_transactions.run_slots().outstanding(),
        0
    );
    assert_eq!(
        read_db(&h, ch(5), PropertyIdentifier::WRITE_STATUS, None).await,
        PropertyValue::Enumerated(WriteStatus::FAILED.to_raw())
    );
    assert_eq!(
        read_db(&h, ch(5), PropertyIdentifier::RELIABILITY, None).await,
        PropertyValue::Enumerated(Reliability::PROCESS_ERROR.to_raw())
    );
}

#[tokio::test(start_paused = true)]
async fn reliability_takes_the_first_failure_to_finish_not_the_first_member() {
    let mut h = start().await;
    let value = PropertyValue::CharacterString("on".into());
    write_channel(&mut h, 5, &value, Some(8)).await.unwrap();
    // AO-1 there reads as a CharacterString, so the value goes on as one.
    answer_reads(&h, 1, &PropertyValue::CharacterString(String::new())).await;
    let (invoke_id, written) = next_request(&h, ao(1)).await;
    assert_eq!(written, [0x73, 0x00, b'o', b'n']);
    // AO-2 here holds a REAL, which a CharacterString can't become: a
    // configuration failure at 100 ms.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(write_status(&mut h, 5).await, WriteStatus::IN_PROGRESS);
    // AO-1, first in the list, fails later, as a process error.
    h.respond(error(invoke_id, ErrorCode::WRITE_ACCESS_DENIED))
        .await;
    assert_eq!(settled(&mut h, 5).await, WriteStatus::FAILED);
    assert_eq!(
        reliability(&mut h, 5).await,
        Reliability::CONFIGURATION_ERROR
    );
}

#[tokio::test(start_paused = true)]
async fn a_device_takes_one_request_from_the_runs_at_a_time() {
    let mut h = start().await;
    // NULL needs no read, so each member is one write; Device 9 gets them
    // one after the other.
    write_channel(&mut h, 10, &PropertyValue::Null, Some(8))
        .await
        .unwrap();
    let mut written = Vec::new();
    while written.len() < 20 {
        let [(invoke_id, request)]: [_; 1] = next_writes(&h).await.try_into().unwrap();
        assert_eq!(h.server.notification_transactions.active_count(), 1);
        written.push(request.object_identifier.instance_number());
        h.respond(ack(invoke_id)).await;
    }
    written.sort_unstable();
    assert_eq!(written, (1..=20).collect::<Vec<_>>());
    assert_eq!(settled(&mut h, 10).await, WriteStatus::SUCCESSFUL);
}

#[tokio::test(start_paused = true)]
async fn the_server_keeps_at_most_thirty_two_run_requests_outstanding() {
    let mut h = start().await;
    // Forty devices across two distributions: they share the server's 32.
    for instance in [11, 12] {
        write_channel(&mut h, instance, &PropertyValue::Null, Some(8))
            .await
            .unwrap();
    }
    let first = next_writes(&h).await;
    assert_eq!(first.len(), 32);
    assert_eq!(h.server.notification_transactions.active_count(), 32);
    let slots = h.server.notification_transactions.run_slots();
    assert_eq!(slots.outstanding(), 32);

    // Each answer lets one more go.
    for (invoke_id, _) in &first[..8] {
        h.respond(ack(*invoke_id)).await;
    }
    let rest = next_writes(&h).await;
    assert_eq!(rest.len(), 8);
    assert_eq!(h.server.notification_transactions.active_count(), 32);
    for (invoke_id, _) in first[8..].iter().chain(&rest) {
        h.respond(ack(*invoke_id)).await;
    }
    for instance in [11, 12] {
        assert_eq!(settled(&mut h, instance).await, WriteStatus::SUCCESSFUL);
    }
    assert_eq!(first.len() + rest.len(), 40);
    let slots = h.server.notification_transactions.run_slots();
    assert_eq!((slots.outstanding(), slots.devices()), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn once_a_device_is_found_silent_its_queued_members_go_unsent() {
    let mut h = start().await;
    let started = tokio::time::Instant::now();
    write_channel(&mut h, 10, &PropertyValue::Null, Some(8))
        .await
        .unwrap();
    // AO-1's write goes four times under one invoke ID with no answer; the
    // nineteen members queued behind it at Device 9 then fail unsent.
    assert_eq!(settled(&mut h, 10).await, WriteStatus::FAILED);
    let took = started.elapsed();
    assert!(
        (Duration::from_secs(12)..Duration::from_millis(12_100)).contains(&took),
        "{took:?}"
    );
    let attempts = sent_writes(&h);
    assert_eq!(attempts.len(), 4);
    assert!(attempts.iter().all(|(id, _)| *id == attempts[0].0));
    assert_eq!(
        reliability(&mut h, 10).await,
        Reliability::COMMUNICATION_FAILURE
    );
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}
