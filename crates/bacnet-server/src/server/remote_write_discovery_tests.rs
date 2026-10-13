//! A write in another device the server has no fresh binding for looks for
//! that device first, with one Who-Is limited to its instance (#1322,
//! Clause 16.10).
//!
//! The harness is `command_remote_write_tests`' without the binding: CMD-1's
//! list 1 writes AO-1 in Device 9, then the local AO-2. Writes are made as a
//! run, or through the run host directly to see the [`RemoteRequestError`]
//! they end with. Who-Is requests are taken from the test transport's send
//! log, and I-Am answers are delivered by hand from the harness peer. The
//! clock is paused, and the server's APDU timeout is the default 3 seconds.
use super::binding_probes::WHO_IS_HOLD_OFF;
use super::command_action_wire_tests::{ao, idle, slot8, state, write, write_pv};
use super::command_remote_write_tests::{
    ack, deliver, device, disable_initiation, remote_write, sent_writes, start_unbound,
};
use super::command_run_stop_tests::{db_flags, db_state};
use super::command_runs::CommandRunner;
use super::cov_wire_test_support::*;
use super::device_bindings::OBSERVED_BINDING_TTL;
use super::test_transport::SentFrame;
use super::*;
use crate::command_lists::RunHost;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_types::constructed::BACnetActionCommand;
use bacnet_types::network_number::NetworkNumber;
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;

/// The APDU timeout: how long a write waits for its device's I-Am.
const WAIT: Duration = Duration::from_secs(3);

/// A Who-Is the server sent: its NPDU destination (none on this network)
/// and its limits.
pub(super) type WhoIs = (Option<NpduAddress>, WhoIsRequest);

/// The Who-Is for Device `instance` alone.
pub(super) fn targeted(instance: u32) -> WhoIsRequest {
    WhoIsRequest {
        range: Some(DeviceInstanceRange::single(instance).unwrap()),
    }
}

/// The destination of a broadcast on remote network `network`.
fn on_network(network: u16) -> Option<NpduAddress> {
    Some(NpduAddress {
        network,
        mac_address: MacAddr::new(),
    })
}

/// The destination of a broadcast on every network.
pub(super) fn everywhere() -> Option<NpduAddress> {
    on_network(0xFFFF)
}

/// Take every Who-Is the server has broadcast.
pub(super) fn who_is_sent(h: &Harness) -> Vec<WhoIs> {
    let log = h.server.test_network().transport().sent();
    let mut frames = log.lock();
    let mut taken = Vec::new();
    frames.retain(|frame| {
        let npdu = frame.decode_npdu();
        match decode_apdu(npdu.payload) {
            Ok(Apdu::UnconfirmedRequest(request))
                if request.service_choice == UnconfirmedServiceChoice::WHO_IS =>
            {
                assert!(frame.broadcast, "a Who-Is goes out as a broadcast");
                let who_is = WhoIsRequest::decode(&request.service_request).unwrap();
                taken.push((npdu.destination, who_is));
                false
            }
            _ => true,
        }
    });
    taken
}

/// Wait for the one Who-Is the server sends next.
pub(super) async fn next_who_is(h: &Harness) -> WhoIs {
    let log = h.server.test_network().transport().sent();
    let sent = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let pushed = log.len() + 1;
            let sent = who_is_sent(h);
            if !sent.is_empty() {
                return sent;
            }
            log.wait_for_len(pushed).await;
        }
    })
    .await
    .expect("a Who-Is");
    let [who_is]: [WhoIs; 1] = sent.try_into().unwrap();
    who_is
}

/// Device `instance`'s I-Am.
pub(super) fn i_am(instance: u32) -> Apdu {
    let mut service = BytesMut::new();
    IAmRequest {
        object_identifier: device(instance),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 260,
    }
    .encode(&mut service);
    Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service.freeze(),
    })
}

/// The WriteProperty frame the server sent.
fn write_frame(h: &Harness) -> SentFrame {
    let unicasts = h.server.test_network().transport().sent().unicasts();
    unicasts
        .into_iter()
        .find(|frame| matches!(frame.apdu(), Apdu::ConfirmedRequest(_)))
        .expect("the WriteProperty frame")
}

/// Address 0x09 on network `network`, behind the harness peer.
fn behind_peer(network: u16) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(&[0x09]),
    }
}

/// Record Device 9's I-Am as heard from the harness peer, routed from
/// `routed`, longer ago than a binding lasts. A host whose monotonic clock
/// started more recently has no such instant: the caller skips there, and
/// `device_bindings_tests` covers the same rules with instants ahead of now.
async fn heard_long_ago(h: &Harness, routed: &NpduAddress) -> bool {
    let age = OBSERVED_BINDING_TTL + Duration::from_secs(60);
    let Some(then) = runtime_clock::now().checked_sub(age) else {
        eprintln!("skipped: the host's monotonic clock is younger than a stale binding");
        return false;
    };
    h.server.device_bindings.write().await.observe_i_am_at(
        device(9),
        &PEER,
        Some(routed),
        then,
        |_| false,
    );
    true
}

/// How a write ended, and when.
type Ended = (Result<(), RemoteRequestError>, TokioInstant);

/// Start writing AO-1 in Device `instance` through the run host.
fn start_write(h: &Harness, instance: u32) -> JoinHandle<Ended> {
    let runner = CommandRunner::for_server(&h.server);
    let command = BACnetActionCommand {
        device_identifier: Some(device(instance)),
        ..write(ao(1), PropertyValue::Real(80.0), 8)
    };
    tokio::spawn(async move {
        let result = runner.write_remote(device(instance), &command).await;
        (result, TokioInstant::now())
    })
}

#[tokio::test(start_paused = true)]
async fn unbound_command_target_answers_a_global_who_is_and_the_write_is_made() {
    let mut h = start_unbound().await;
    write_pv(&mut h, 1, 1).await.unwrap();
    // A device never heard from is asked on every network, alone; no write
    // goes yet.
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    assert!(sent_writes(&h).is_empty());
    assert_eq!(state(&mut h, 1).await, (true, false));

    // Its I-Am binds Device 9, and the write goes to the peer that sent it.
    deliver(&h, &i_am(9), &PEER, None).await;
    let invoke_id = remote_write(&h).await;
    assert_eq!(write_frame(&h).mac.as_slice(), PEER);
    h.respond(ack(invoke_id)).await;
    idle(&h, 1).await;
    assert_eq!(db_flags(&h, 1).await, [true, true]);
    assert_eq!(state(&mut h, 1).await, (false, true));

    // The binding stands, so the next run asks nothing.
    write_pv(&mut h, 1, 1).await.unwrap();
    let invoke_id = remote_write(&h).await;
    h.respond(ack(invoke_id)).await;
    idle(&h, 1).await;
    assert!(who_is_sent(&h).is_empty());
}

#[tokio::test(start_paused = true)]
async fn silent_unbound_target_fails_after_the_apdu_timeout_with_one_who_is() {
    let h = start_unbound().await;
    let started = TokioInstant::now();
    let (result, ended) = start_write(&h, 9).await.unwrap();
    assert_eq!(result, Err(RemoteRequestError::Undiscovered));
    assert_eq!(ended - started, WAIT);
    assert_eq!(who_is_sent(&h), [(everywhere(), targeted(9))]);
    assert!(sent_writes(&h).is_empty());
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_wait_for_an_i_am_counts_from_the_who_is_send() {
    let h = start_unbound().await;
    let link = h.server.test_network().transport().handle();
    link.block_next_send();
    let started = TokioInstant::now();
    let write = start_write(&h, 9);
    // The link holds the Who-Is for a second before it goes out.
    link.wait_blocked().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    link.release_sends(1);
    let (result, ended) = write.await.unwrap();
    assert_eq!(result, Err(RemoteRequestError::Undiscovered));
    assert_eq!(ended - started, Duration::from_secs(1) + WAIT);
    assert_eq!(who_is_sent(&h).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn silent_unbound_command_target_fails_and_the_rest_of_the_list_runs() {
    let mut h = start_unbound().await;
    let started = TokioInstant::now();
    write_pv(&mut h, 1, 1).await.unwrap();
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    idle(&h, 1).await;
    assert!(started.elapsed() >= WAIT);
    assert_eq!(db_flags(&h, 1).await, [false, true]);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    assert_eq!(state(&mut h, 1).await, (false, false));
    assert!(sent_writes(&h).is_empty());
    assert!(who_is_sent(&h).is_empty());
}

#[tokio::test(start_paused = true)]
async fn no_who_is_while_dcc_restricts_initiation_or_for_the_wildcard_device() {
    let mut h = start_unbound().await;
    disable_initiation(&h);
    let started = TokioInstant::now();
    let (result, ended) = start_write(&h, 9).await.unwrap();
    assert_eq!(result, Err(RemoteRequestError::Disabled));
    assert_eq!(ended, started);
    write_pv(&mut h, 1, 1).await.unwrap();
    idle(&h, 1).await;
    assert_eq!(db_flags(&h, 1).await, [false, true]);
    assert!(who_is_sent(&h).is_empty());
    assert!(sent_writes(&h).is_empty());

    // With initiation allowed again, the same write asks for Device 9, but a
    // Who-Is for the wildcard instance would call on unconfigured devices,
    // so a write naming it fails at once.
    h.server.comm_state.set_for_test(DccState::Enable);
    let wildcard = ObjectIdentifier::WILDCARD_INSTANCE;
    let (result, _) = start_write(&h, wildcard).await.unwrap();
    assert_eq!(result, Err(RemoteRequestError::Unbound));
    assert!(who_is_sent(&h).is_empty());
    let write = start_write(&h, 9);
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    deliver(&h, &i_am(9), &PEER, None).await;
    let invoke_id = remote_write(&h).await;
    h.respond(ack(invoke_id)).await;
    assert_eq!(write.await.unwrap().0, Ok(()));
}

#[tokio::test(start_paused = true)]
async fn concurrent_writes_to_an_unbound_device_share_one_who_is() {
    let h = start_unbound().await;
    let first = start_write(&h, 9);
    let second = start_write(&h, 9);
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    deliver(&h, &i_am(9), &PEER, None).await;
    // The one I-Am lets both writes go, each under its own invoke ID.
    let log = h.server.test_network().transport().sent();
    let sent = tokio::time::timeout(Duration::from_secs(1), async {
        let mut sent = Vec::new();
        loop {
            let pushed = log.len() + 1;
            sent.extend(sent_writes(&h));
            if sent.len() == 2 {
                return sent;
            }
            log.wait_for_len(pushed).await;
        }
    })
    .await
    .expect("both WriteProperty requests");
    assert_ne!(sent[0].0, sent[1].0);
    for (invoke_id, _) in sent {
        h.respond(ack(invoke_id)).await;
    }
    assert_eq!(first.await.unwrap().0, Ok(()));
    assert_eq!(second.await.unwrap().0, Ok(()));
    assert!(who_is_sent(&h).is_empty());
}

#[tokio::test(start_paused = true)]
async fn absent_device_gets_one_who_is_per_hold_off() {
    let h = start_unbound().await;
    let started = TokioInstant::now();
    // Two writes inside the wait share the one Who-Is and its deadline.
    let first = start_write(&h, 9);
    let second = start_write(&h, 9);
    for write in [first, second] {
        let (result, ended) = write.await.unwrap();
        assert_eq!(result, Err(RemoteRequestError::Undiscovered));
        assert_eq!(ended - started, WAIT);
    }
    assert_eq!(who_is_sent(&h).len(), 1);

    // Until a minute after it went out, the next write sends nothing and
    // fails at once.
    tokio::time::advance(WHO_IS_HOLD_OFF - WAIT - Duration::from_millis(1)).await;
    let asked = TokioInstant::now();
    let (result, ended) = start_write(&h, 9).await.unwrap();
    assert_eq!(result, Err(RemoteRequestError::Unbound));
    assert_eq!(ended, asked);
    assert!(who_is_sent(&h).is_empty());

    // Then the device is asked again.
    tokio::time::advance(Duration::from_millis(1)).await;
    let asked = TokioInstant::now();
    let (result, ended) = start_write(&h, 9).await.unwrap();
    assert_eq!(result, Err(RemoteRequestError::Undiscovered));
    assert_eq!(ended - asked, WAIT);
    assert_eq!(who_is_sent(&h), [(everywhere(), targeted(9))]);
}

#[tokio::test(start_paused = true)]
async fn who_is_for_a_stale_routed_binding_goes_to_its_network() {
    let h = start_unbound().await;
    // Device 9 was last heard at 0x09 on network 5, behind the harness peer.
    let routed = behind_peer(5);
    if !heard_long_ago(&h, &routed).await {
        return;
    }
    let write = start_write(&h, 9);
    assert_eq!(next_who_is(&h).await, (on_network(5), targeted(9)));
    deliver(&h, &i_am(9), &PEER, Some(routed.clone())).await;
    let invoke_id = remote_write(&h).await;
    let request = write_frame(&h);
    assert_eq!(request.mac.as_slice(), PEER);
    assert_eq!(request.decode_npdu().destination, Some(routed.clone()));
    deliver(&h, &ack(invoke_id), &PEER, Some(routed)).await;
    assert_eq!(write.await.unwrap().0, Ok(()));
}

#[tokio::test(start_paused = true)]
async fn a_fruitless_who_is_on_the_last_network_drops_it_and_the_next_asks_everywhere() {
    let h = start_unbound().await;
    if !heard_long_ago(&h, &behind_peer(5)).await {
        return;
    }
    let started = TokioInstant::now();
    let (result, ended) = start_write(&h, 9).await.unwrap();
    assert_eq!(result, Err(RemoteRequestError::Undiscovered));
    assert_eq!(ended - started, WAIT);
    assert_eq!(who_is_sent(&h), [(on_network(5), targeted(9))]);
    // The stale observation is gone, so network 5 isn't asked again.
    assert_eq!(h.server.device_bindings.read().await.len(), 0);

    // After the hold-off the device is asked everywhere, and found on
    // network 6 now.
    tokio::time::advance(WHO_IS_HOLD_OFF - WAIT).await;
    let write = start_write(&h, 9);
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    let moved = behind_peer(6);
    deliver(&h, &i_am(9), &PEER, Some(moved.clone())).await;
    let invoke_id = remote_write(&h).await;
    assert_eq!(
        write_frame(&h).decode_npdu().destination,
        Some(moved.clone())
    );
    deliver(&h, &ack(invoke_id), &PEER, Some(moved)).await;
    assert_eq!(write.await.unwrap().0, Ok(()));
}

#[tokio::test(start_paused = true)]
async fn stale_binding_on_this_network_by_number_is_asked_with_no_dnet() {
    let h = start_unbound().await;
    // This device's network is number 5, which Device 9's stale routed
    // observation names: the Who-Is goes as a local broadcast (#1365).
    let this_network = NetworkNumber::configured(5).unwrap();
    h.server
        .test_network()
        .local_network_number()
        .publish(this_network);
    if !heard_long_ago(&h, &behind_peer(5)).await {
        return;
    }
    let write = start_write(&h, 9);
    assert_eq!(next_who_is(&h).await, (None, targeted(9)));
    deliver(&h, &i_am(9), &PEER, None).await;
    let invoke_id = remote_write(&h).await;
    let request = write_frame(&h);
    assert_eq!(request.mac.as_slice(), PEER);
    assert_eq!(request.decode_npdu().destination, None);
    h.respond(ack(invoke_id)).await;
    assert_eq!(write.await.unwrap().0, Ok(()));
}

#[tokio::test(start_paused = true)]
async fn stop_during_a_who_is_wait_ends_the_run_with_nothing_made() {
    let mut h = start_unbound().await;
    write_pv(&mut h, 1, 1).await.unwrap();
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    h.server.stop().await.unwrap();
    assert_eq!(db_state(&h, 1).await, (false, false));
    assert_eq!(db_flags(&h, 1).await, [false, false]);
    assert!(sent_writes(&h).is_empty());
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}
