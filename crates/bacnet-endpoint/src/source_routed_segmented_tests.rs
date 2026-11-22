//! A segmented request the node audits, to Device 999 behind the router
//! (#2617). The client hands it to the session where it would have gone
//! segmented, and its caller's turn at the station and routed path go with
//! it: a refusal frees both, and the turn ends with the request, not with
//! its caller.
//!
//! After a routed request of several segments the session holds that path
//! for the APDU timeout times (retries + 1), so these sessions time out in
//! 3 s, keeping that hold shorter than the waits measured here.
use super::*;

/// How far apart the session sends requests to one station.
const INTERVAL: Duration = Duration::from_secs(5);

/// The site, its client-only session pacing requests to one station
/// `INTERVAL` apart and auditing writes.
async fn paced_site() -> Site {
    let config = SessionConfig {
        apdu_timeout_ms: 3_000,
        apdu_retries: 0,
        min_request_interval_ms: 5_000,
        ..SessionConfig::default()
    };
    Site::start_with(
        SessionRole::ClientOnly,
        false,
        &[AuditOperation::WRITE],
        config,
    )
    .await
}

type Client = Arc<bacnet_client::client::BACnetClient<LoopbackTransport>>;

/// A 400-octet file write to Device 999 behind the router, in a task of
/// its own: segmented, since the routed path's conservative limit is far
/// smaller.
fn write(client: &Client) -> tokio::task::JoinHandle<Result<Bytes, Error>> {
    let client = Arc::clone(client);
    tokio::spawn(async move {
        let station = bacnet_client::client::Station::routed(&ROUTER_NEAR, REMOTE, &STATION);
        let access = bacnet_services::file::FileWriteAccessMethod::Stream {
            file_start_position: 0,
            file_data: vec![7; 400],
        };
        client
            .atomic_write_file(&station, oid(ObjectType::FILE, 3), access)
            .await
    })
}

/// Answer one segment of the write: true when it was the last.
async fn answer(site: &Site, received: &ReceivedApdu) -> bool {
    let Apdu::ConfirmedRequest(pdu) = decode_apdu(received.apdu.clone()).unwrap() else {
        panic!("a segment of the write")
    };
    assert!(pdu.segmented, "the write goes segmented");
    let answer = if pdu.more_follows {
        Apdu::SegmentAck(bacnet_encoding::apdu::SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id: pdu.invoke_id,
            sequence_number: pdu.sequence_number.unwrap(),
            actual_window_size: 1,
        })
    } else {
        let mut ack = BytesMut::new();
        bacnet_encoding::primitives::encode_ctx_signed(&mut ack, 0, 0);
        Apdu::ComplexAck(bacnet_encoding::apdu::ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: pdu.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
            service_ack: ack.freeze(),
        })
    };
    site.send(answer, false).await;
    !pdu.more_follows
}

/// Answer the write from its segment `first` on: how many segments it took.
async fn finish(site: &mut Site, first: ReceivedApdu) -> usize {
    let mut received = first;
    let mut segments = 1;
    while !answer(site, &received).await {
        received = site.receive().await;
        segments += 1;
    }
    segments
}

/// Whether the request that went `went` after `since` waited the interval
/// from it. Paused time fires the pacer's timer at its deadline; the 100 ms
/// allows for the test reading the clock a little after the turn ended.
fn paced(went: Duration) -> bool {
    went >= INTERVAL - Duration::from_millis(100)
}

/// A segmented audited write refused at admission, after its caller's turn
/// and routed path went with it: the turn ends at the refusal, so the next
/// write waits the interval from then, and the routed path is free, so the
/// next write's runner takes it and goes.
#[tokio::test(start_paused = true)]
async fn a_refused_segmented_audit_frees_its_turn_and_path() {
    let mut site = paced_site().await;
    let full: Client = Arc::new(site.session.client().unwrap());
    let seat = site
        .session
        .ingress
        .as_ref()
        .unwrap()
        .server_seat(Arc::clone(&site.session.coordinator))
        .unwrap();
    let source = Arc::clone(site.session.source_audit.as_ref().unwrap());
    let db = Arc::clone(site.session.database.as_ref().unwrap());
    let guard = db.write().await;
    let refused = write(&full);
    eventually(WAIT, "the write holds a slot", async || {
        source.available_operations() != 64
    })
    .await;
    let claimed = tokio::time::Instant::now();
    tokio::time::sleep(Duration::from_secs(10)).await;
    seat.dcc.set(true);
    drop(guard);
    assert!(matches!(
        timeout(WAIT, refused).await.unwrap().unwrap(),
        Err(Error::InitiationDisabled { sent: false })
    ));
    let ended = tokio::time::Instant::now();
    seat.dcc.set(false);
    assert!(
        site.rx.try_recv().is_err(),
        "the refused write sent nothing"
    );
    let next = write(&full);
    let first = timeout(Duration::from_secs(20), site.rx.recv())
        .await
        .expect("the next write goes: the routed path is free")
        .unwrap();
    let went = tokio::time::Instant::now();
    assert!(
        paced(went - ended),
        "the next went {:?} after the refusal, {:?} after the turn was claimed",
        went - ended,
        went - claimed
    );
    assert!(finish(&mut site, first).await > 1);
    assert!(timeout(WAIT, next).await.unwrap().unwrap().is_ok());
    let (record, _) = notification(&site.receive().await, false);
    assert_eq!(record.operation, AuditOperation::WRITE);
    drop(full);
    drop(seat);
    site.stop().await;
}

/// The caller's turn travels with a segmented audited write: its caller
/// goes away after the first segment, and the next write still waits for
/// the first one's end and then the interval.
#[tokio::test(start_paused = true)]
async fn the_turn_travels_with_a_segmented_audited_write() {
    let mut site = paced_site().await;
    let full: Client = Arc::new(site.session.client().unwrap());
    let first = write(&full);
    let segment = site.receive().await;
    first.abort();
    let _ = first.await;
    let next = write(&full);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        site.rx.try_recv().is_err(),
        "the next waits while the first runs"
    );
    assert!(finish(&mut site, segment).await > 1);
    let ended = tokio::time::Instant::now();
    let (record, _) = notification(&site.receive().await, false);
    assert_eq!(record.operation, AuditOperation::WRITE);
    let second = timeout(Duration::from_secs(20), site.rx.recv())
        .await
        .expect("the next write goes")
        .unwrap();
    let went = tokio::time::Instant::now();
    assert!(
        paced(went - ended),
        "the next went {:?} after the first ended",
        went - ended
    );
    assert!(finish(&mut site, second).await > 1);
    assert!(timeout(WAIT, next).await.unwrap().unwrap().is_ok());
    drop(full);
    site.stop().await;
}
