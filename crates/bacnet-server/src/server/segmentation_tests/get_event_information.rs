use std::borrow::Cow;

use bacnet_encoding::primitives::encode_timestamp_choice;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::GetEventInformationRequest;
use bacnet_types::enums::EventState;
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

use super::*;

const INVOKE_ID: u8 = 77;

struct LargeSummaryObject {
    oid: ObjectIdentifier,
    timestamps: Vec<PropertyValue>,
}

impl LargeSummaryObject {
    fn new() -> Self {
        let timestamps = (1..=3)
            .map(|hour| {
                let timestamp = BACnetTimeStamp::DateTime {
                    date: Date {
                        year: 126,
                        month: 9,
                        day: hour,
                        day_of_week: hour,
                    },
                    time: Time {
                        hour,
                        minute: 2,
                        second: 3,
                        hundredths: 4,
                    },
                };
                let mut encoded = BytesMut::new();
                encode_timestamp_choice(&mut encoded, &timestamp).unwrap();
                PropertyValue::ApplicationData(encoded.to_vec())
            })
            .collect();
        Self {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            timestamps,
        }
    }
}

impl BACnetObject for LargeSummaryObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "LARGE-SUMMARY"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw()))
            }
            p if p == PropertyIdentifier::ACKED_TRANSITIONS
                || p == PropertyIdentifier::EVENT_ENABLE =>
            {
                Ok(PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![0xe0],
                })
            }
            p if p == PropertyIdentifier::EVENT_TIME_STAMPS => {
                Ok(PropertyValue::List(self.timestamps.clone()))
            }
            p if p == PropertyIdentifier::NOTIFY_TYPE => Ok(PropertyValue::Enumerated(0)),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(42)),
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFICATION_CLASS,
        ])
    }
}

fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(LargeSummaryObject::new())).unwrap();
    let mut class = NotificationClass::new(99, "NC-99").unwrap();
    class.notification_class = 42;
    class.priority = [1, 100, 255];
    db.add(Box::new(class)).unwrap();
    db
}

fn service_request() -> Bytes {
    let mut encoded = BytesMut::new();
    GetEventInformationRequest {
        last_received_object_identifier: None,
    }
    .encode(&mut encoded);
    encoded.freeze()
}

fn full_service_ack() -> Bytes {
    let mut encoded = BytesMut::new();
    handlers::handle_get_event_information(&database(), &service_request(), &mut encoded).unwrap();
    encoded.freeze()
}

fn unsegmented_apdu_len(service_ack: Bytes) -> usize {
    let mut encoded = BytesMut::new();
    encode_apdu(
        &mut encoded,
        &Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: INVOKE_ID,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::GET_EVENT_INFORMATION,
            service_ack,
        }),
    )
    .unwrap();
    encoded.len()
}

async fn dispatch(
    segmentation: Segmentation,
    client_accepts_segmented: bool,
    client_max_apdu: u16,
    local_max_apdu: u32,
) -> (
    SentFrames,
    Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
    MacAddr,
) {
    let sent = SentFrames::default();
    let network = Arc::new(NetworkLayer::new(RecordingTransport::new(Arc::clone(
        &sent,
    ))));
    let db = Arc::new(RwLock::new(database()));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let cov_in_flight = Arc::new(Semaphore::new(1));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let notification_transactions = NotificationTransactions::new();
    let confirmed_request_tracker = Arc::new(ConfirmedRequestTracker::default());
    let device_bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    let comm_state = Arc::new(AtomicU8::new(0));
    let dcc_timer = Arc::new(Mutex::new(None::<JoinHandle<()>>));
    let config = ServerConfig {
        max_apdu_length: local_max_apdu,
        segmentation_supported: segmentation,
        ..ServerConfig::default()
    };
    let source_mac = test_mac(41);

    BACnetServer::<RecordingTransport>::handle_confirmed_request(
        &db,
        &network,
        &cov_table,
        &seg_ack_senders,
        &seg_send_permits,
        &cov_in_flight,
        &server_tsm,
        &notification_transactions,
        &confirmed_request_tracker,
        &device_bindings,
        &comm_state,
        &dcc_timer,
        &config,
        source_mac.as_slice(),
        None,
        ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: client_accepts_segmented,
            max_segments: None,
            max_apdu_length: client_max_apdu,
            invoke_id: INVOKE_ID,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::GET_EVENT_INFORMATION,
            service_request: service_request(),
        },
        None,
    )
    .await;

    (sent, seg_ack_senders, source_mac)
}

#[tokio::test]
async fn first_summary_over_unsegmented_budget_uses_existing_segmentation_abort() {
    assert!(unsegmented_apdu_len(full_service_ack()) > 50);
    for (client_max_apdu, local_max_apdu) in [(50, 1476), (480, 50)] {
        let (sent, _, _) =
            dispatch(Segmentation::NONE, false, client_max_apdu, local_max_apdu).await;
        wait_for_sent_len(&sent, 1).await;
        assert_eq!(sent_count(&sent), 1);
        assert_eq!(
            abort_reason(&sent, 0),
            AbortReason::SEGMENTATION_NOT_SUPPORTED
        );
    }
}

#[tokio::test]
async fn segmentation_capable_dispatch_retains_the_complete_service_ack() {
    let expected = full_service_ack();
    assert!(unsegmented_apdu_len(expected.clone()) > 50);
    let (sent, seg_ack_senders, source_mac) = dispatch(Segmentation::BOTH, true, 50, 1476).await;
    let key = segmented_transaction_key(&source_mac, None, INVOKE_ID);
    let mut reconstructed = BytesMut::new();
    let mut index = 0usize;

    loop {
        wait_for_sent_len(&sent, index + 1).await;
        let Apdu::ComplexAck(ack) = decoded_sent_apdu(&sent, index) else {
            panic!("expected segmented ComplexAck");
        };
        assert!(ack.segmented);
        assert_eq!(
            ack.service_choice,
            ConfirmedServiceChoice::GET_EVENT_INFORMATION
        );
        assert_eq!(ack.sequence_number, Some(index as u8));
        reconstructed.extend_from_slice(&ack.service_ack);
        if !ack.more_follows {
            break;
        }
        send_segment_ack(
            &seg_ack_senders,
            &key,
            segment_ack(INVOKE_ID, false, index as u8),
        )
        .await;
        index += 1;
    }

    assert_eq!(reconstructed.freeze(), expected);
}
