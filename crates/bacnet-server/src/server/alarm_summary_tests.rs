use std::borrow::Cow;

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::traits::BACnetObject;

use super::*;

struct MalformedAlarmObject {
    oid: ObjectIdentifier,
}

impl BACnetObject for MalformedAlarmObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "MALFORMED-ALARM"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Boolean(true)),
            PropertyIdentifier::NOTIFY_TYPE => {
                Ok(PropertyValue::Enumerated(NotifyType::ALARM.to_raw()))
            }
            PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xe0],
            }),
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
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::ACKED_TRANSITIONS,
        ])
    }
}

#[tokio::test]
async fn projection_operational_problem_dispatches_error_apdu() {
    let mut database = ObjectDatabase::new();
    database
        .add(Box::new(MalformedAlarmObject {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        }))
        .unwrap();
    let db = Arc::new(RwLock::new(database));
    let network = Arc::new(NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    )));
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
    let confirmed = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 0x50,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::GET_ALARM_SUMMARY,
        service_request: Bytes::new(),
    };
    let (tx, rx) = oneshot::channel();

    BACnetServer::<BipTransport>::handle_confirmed_request(
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
        &ServerConfig::default(),
        &MacAddr::from_slice(&[1]),
        None,
        confirmed,
        Some(tx),
    )
    .await;

    let response = decode_apdu(decode_npdu(rx.await.unwrap()).unwrap().payload).unwrap();
    let Apdu::Error(error) = response else {
        panic!("expected GetAlarmSummary Error APDU");
    };
    assert_eq!(error.invoke_id, 0x50);
    assert_eq!(
        error.service_choice,
        ConfirmedServiceChoice::GET_ALARM_SUMMARY
    );
    assert_eq!(error.error_class, ErrorClass::DEVICE);
    assert_eq!(error.error_code, ErrorCode::OPERATIONAL_PROBLEM);
}
