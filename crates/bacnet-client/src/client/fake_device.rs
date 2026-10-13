//! A scripted device across a loopback pair, answering each confirmed request
//! from a closure, for client tests that need whole exchanges.
use std::sync::{Arc, Mutex as StdMutex};

use bacnet_encoding::apdu::{self, encode_apdu, Apdu, ComplexAck, ErrorPdu};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bytes::{Bytes, BytesMut};
use tokio::task::JoinHandle;

use super::BACnetClient;

pub(crate) const CLIENT_MAC: [u8; 1] = [0x01];
pub(crate) const DEVICE_MAC: [u8; 1] = [0x02];

/// What the device answers one request with: the service-ack octets, or an
/// Error PDU's class and code.
pub(crate) type Answer = Result<Vec<u8>, (ErrorClass, ErrorCode)>;

/// Every confirmed request the device received: its service and parameters.
pub(crate) type RequestLog = Arc<StdMutex<Vec<(ConfirmedServiceChoice, Vec<u8>)>>>;

pub(crate) struct FakeDevice {
    pub(crate) requests: RequestLog,
    task: JoinHandle<()>,
}

impl Drop for FakeDevice {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeDevice {
    /// How many confirmed requests of `service` arrived.
    pub(crate) fn count(&self, service: ConfirmedServiceChoice) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(choice, _)| *choice == service)
            .count()
    }
}

/// A started client and a device at [`DEVICE_MAC`] answering with `answer`.
pub(crate) async fn client_with_device<F>(
    mut answer: F,
) -> (BACnetClient<LoopbackTransport>, FakeDevice)
where
    F: FnMut(ConfirmedServiceChoice, &[u8]) -> Answer + Send + 'static,
{
    let (client_transport, mut device_transport) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), DEVICE_MAC.to_vec());
    let mut inbound = device_transport.start().await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(1_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    let requests = RequestLog::default();
    let log = Arc::clone(&requests);
    let task = tokio::spawn(async move {
        while let Some(received) = inbound.recv().await {
            let Ok(npdu) = decode_npdu(received.npdu) else {
                continue;
            };
            let Ok(Apdu::ConfirmedRequest(request)) = apdu::decode_apdu(npdu.payload) else {
                continue;
            };
            log.lock()
                .unwrap()
                .push((request.service_choice, request.service_request.to_vec()));
            let reply = match answer(request.service_choice, &request.service_request) {
                Ok(service_ack) => Apdu::ComplexAck(ComplexAck {
                    segmented: false,
                    more_follows: false,
                    invoke_id: request.invoke_id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: request.service_choice,
                    service_ack: Bytes::from(service_ack),
                }),
                Err((error_class, error_code)) => Apdu::Error(ErrorPdu {
                    invoke_id: request.invoke_id,
                    service_choice: request.service_choice,
                    error_class,
                    error_code,
                    error_data: Bytes::new(),
                }),
            };
            let mut apdu_buf = BytesMut::new();
            encode_apdu(&mut apdu_buf, &reply).unwrap();
            let mut npdu_buf = BytesMut::new();
            encode_npdu(
                &mut npdu_buf,
                &Npdu {
                    payload: apdu_buf.freeze(),
                    ..Npdu::default()
                },
            )
            .unwrap();
            if device_transport
                .send_unicast(&npdu_buf, &CLIENT_MAC)
                .await
                .is_err()
            {
                break;
            }
        }
    });
    (client, FakeDevice { requests, task })
}
