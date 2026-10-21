//! A flood of malformed unconfirmed requests, or of I-Ams from new devices
//! once the binding table is full, warns through the server's throttles
//! (#1950): once per interval each, the rest at debug.

use super::super::request_services::InboundWarnings;
use super::*;
use bacnet_transport::warn_throttle::WARNING_INTERVAL;

#[tokio::test(start_paused = true)]
async fn malformed_requests_and_a_full_binding_table_warn_once_per_interval() {
    let network = Arc::new(NetworkLayer::new(transport()));
    let warnings = Arc::new(InboundWarnings::default());
    let bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    {
        let mut table = bindings.write().await;
        let now = crate::runtime_clock::now();
        for instance in 0..MAX_DEVICE_BINDINGS as u32 {
            table.observe_i_am_at(device(instance), LOCAL_PEER, None, now, no_broadcast);
        }
    }
    let services = UnconfirmedServices {
        device_bindings: Arc::clone(&bindings),
        warnings: Arc::clone(&warnings),
        ..UnconfirmedServices::for_test(network, ServerConfig::default())
    };
    let malformed = |service_choice| UnconfirmedRequestPdu {
        service_choice,
        service_request: Bytes::from_static(&[0xFF]),
    };
    for round in 0..3u32 {
        for service_choice in [
            UnconfirmedServiceChoice::I_AM,
            UnconfirmedServiceChoice::WHO_IS,
            UnconfirmedServiceChoice::WHO_HAS,
        ] {
            BACnetServer::<TestTransport>::handle_unconfirmed_request(
                &services,
                malformed(service_choice),
                &received(UPDATED_PEER, None),
            )
            .await;
        }
        let fresh = device(MAX_DEVICE_BINDINGS as u32 + round);
        BACnetServer::<TestTransport>::handle_unconfirmed_request(
            &services,
            i_am_request(fresh),
            &received(UPDATED_PEER, None),
        )
        .await;
    }
    assert_eq!(
        warnings.decode.note(),
        None,
        "the first decode failure warned"
    );
    assert_eq!(
        warnings.binding_capacity.note(),
        None,
        "the first I-Am warned"
    );
    tokio::time::advance(WARNING_INTERVAL).await;
    assert_eq!(
        warnings.decode.note(),
        Some(10),
        "eight held back, the check above, and this one"
    );
    assert_eq!(
        warnings.binding_capacity.note(),
        Some(4),
        "two held back, the check above, and this one"
    );
}
