//! The endpoint client's pacing on the capture link, on paused time
//! (#1542): an audited read waits for its turn before it takes an admission
//! permit or the database, a stopped or dropped session ends the wait at
//! once and lets go of the database, and a destination routed to this
//! network shares the lane of the station it names while one on another
//! network has its own.
use super::*;
use tokio::time::Instant;

/// Minutes: a waiting request stays waiting for the whole test.
const LONG_MS: u64 = 300_000;

fn direct_to_peer() -> EndpointApduDestination {
    EndpointApduDestination::Direct {
        destination_mac: mac(PEER),
    }
}

/// A started ClientOnly source session reporting to Device 999 at `SINK`,
/// pacing its requests `interval_ms` apart, that has made one audited read
/// of the peer.
async fn paced_source(interval_ms: u64) -> Endpoint {
    let recipient = BACnetRecipient::Device(oid(ObjectType::DEVICE, 999));
    let mut endpoint = unstarted_source(SessionRole::ClientOnly, recipient, None);
    endpoint.session.client_config.min_request_interval_ms = interval_ms;
    endpoint.start(false).await;
    audited_read(&mut endpoint, direct_to_peer(), SINK).await;
    endpoint
}

/// Start a second audited read of the peer and run until the runtime is
/// idle: the read is waiting for its turn.
async fn waiting_read(
    endpoint: &Endpoint,
) -> tokio::task::JoinHandle<Result<ReadPropertyACK, Error>> {
    let read = endpoint.read(direct_to_peer());
    tokio::time::sleep(Duration::from_secs(1)).await;
    read
}

#[tokio::test(start_paused = true)]
async fn a_waiting_audited_read_holds_no_permit_or_database_lock() {
    let mut endpoint = paced_source(LONG_MS).await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    // The database is held here, so a read that took it would block.
    let writing = Arc::clone(&db).write_owned().await;
    let read = waiting_read(&endpoint).await;
    assert_eq!(source.available_operations(), 64, "no permit taken");
    assert!(endpoint.sent.try_recv().is_err(), "nothing sent");
    // The read goes at its turn, once the database is free.
    drop(writing);
    tokio::time::advance(Duration::from_millis(LONG_MS)).await;
    let (route, request) = endpoint.request().await;
    assert_eq!(route, local(PEER));
    endpoint
        .deliver_apdu(ack(&request, 5), mac(PEER), None)
        .await;
    bounded(read).await.unwrap().unwrap();
    drop(source);
    endpoint.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_waiting_audited_read_fails_at_once_when_the_session_stops_or_drops() {
    for stop in [true, false] {
        let mut endpoint = paced_source(LONG_MS).await;
        let weak = Arc::downgrade(endpoint.session.database.as_ref().unwrap());
        let read = waiting_read(&endpoint).await;
        let endpoint = if stop {
            bounded(endpoint.session.stop()).await.unwrap();
            Some(endpoint)
        } else {
            drop(endpoint);
            None
        };
        let error = bounded(read).await.unwrap().unwrap_err();
        assert!(
            matches!(&error, Error::Encoding(message) if message == "endpoint shutdown"),
            "stopped {stop}: {error:?}"
        );
        // The waiter no longer holds the source, so nothing of the session
        // keeps the database once the session itself is gone.
        drop(endpoint);
        bounded(async {
            while weak.strong_count() > 0 {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_destination_routed_to_this_network_shares_the_stations_lane() {
    let mut endpoint = Endpoint::new(SessionRole::ClientOnly);
    let db = crate::DeviceIdentity::new(123, 42)
        .unwrap()
        .build_database()
        .unwrap();
    endpoint.session = endpoint.session.with_database(db);
    endpoint.session.client_config.min_request_interval_ms = 1_000;
    endpoint.start(true).await;
    let read = endpoint.read(direct_to_peer());
    let (_, request) = endpoint.request().await;
    endpoint
        .deliver_apdu(ack(&request, 5), mac(PEER), None)
        .await;
    bounded(read).await.unwrap().unwrap();
    let answered = Instant::now();
    // The same MAC on another network is another device: it goes at once.
    let read = endpoint.read(routed_read(REMOTE_NETWORK));
    let (route, request) = endpoint.request().await;
    assert_eq!(route, routed(mac(ROUTER), REMOTE_NETWORK));
    assert_eq!(Instant::now(), answered, "another network's lane");
    endpoint
        .deliver_apdu(ack(&request, 5), mac(ROUTER), peer_on(REMOTE_NETWORK))
        .await;
    bounded(read).await.unwrap().unwrap();
    // Routed to this network, it is the station read first: it waits for
    // the interval from that read's answer.
    let read = endpoint.read(routed_read(THIS_NETWORK));
    let (route, request) = endpoint.request().await;
    assert_eq!(route, local(PEER));
    assert_eq!(Instant::now() - answered, Duration::from_secs(1));
    endpoint
        .deliver_apdu(ack(&request, 5), mac(PEER), None)
        .await;
    bounded(read).await.unwrap().unwrap();
    endpoint.stop().await;
}
