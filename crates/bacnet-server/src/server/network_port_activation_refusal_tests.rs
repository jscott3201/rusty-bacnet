//! ReinitializeDevice when pending Network Port values are refused (#1578):
//! PROPERTY / INVALID_CONFIGURATION_DATA (Clause 16.4.1.3.1), before the
//! handler when the dry run refuses them, and after it on a late race.
//! The port's tables are a fake whose answers the test sets.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::network_port_mode_tests::port_oid;
use super::*;
use crate::server::reinitialize::{AfterReply, Requester, response};
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::bip_port::{BbmdChanges, BbmdTables, BipPortMode};
use bacnet_types::constructed::{BACnetBDTEntry, BACnetFDTEntry};
use bacnet_types::enums::{PropertyIdentifier as P, ReinitializedState};

/// Tables that refuse a check or an activation when told to.
#[derive(Default)]
struct Tables {
    refuse_check: AtomicBool,
    refuse_activation: AtomicBool,
    activated: AtomicUsize,
}

fn refusal() -> Error {
    Error::protocol(ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
}

impl BbmdTables for Tables {
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry> {
        Vec::new()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        false
    }
    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry> {
        Vec::new()
    }
    fn check_broadcast_distribution_table(&self, _: &[BACnetBDTEntry]) -> Result<(), Error> {
        if self.refuse_check.load(Ordering::SeqCst) {
            return Err(refusal());
        }
        Ok(())
    }
    fn activate(&self, _: &BbmdChanges) -> Result<(), Error> {
        if self.refuse_activation.load(Ordering::SeqCst) {
            return Err(refusal());
        }
        self.activated.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn can_change(&self) -> bool {
        true
    }
}

/// A database with a BBMD port over `tables` holding a pending BDT, and the
/// lease that keeps the port registered.
fn database(tables: &Arc<Tables>) -> (RwLock<ObjectDatabase>, Arc<()>) {
    let mut db = ObjectDatabase::new();
    let config = BipPortConfig {
        ip_address: [127, 0, 0, 1],
        ..Default::default()
    };
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "bbmd", config).unwrap(),
    ))
    .unwrap();
    let (_, lease) = db
        .reserve_bip_port_internal(port_oid(), [127, 0, 0, 1], 47808)
        .unwrap();
    let view: Arc<dyn BbmdTables> = tables.clone();
    let mode = BipPortMode::Bbmd { tables: Some(view) };
    db.publish_bip_port_internal(port_oid(), [127, 0, 0, 1], 47808, 1476, mode)
        .unwrap();
    db.set_network_port_activation_internal(port_oid(), true)
        .unwrap();
    db.get_mut(&port_oid())
        .unwrap()
        .write_property(
            P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
            None,
            PropertyValue::ApplicationData(Vec::new()),
            None,
        )
        .unwrap();
    (RwLock::new(db), lease)
}

async fn reinitialize(
    db: &RwLock<ObjectDatabase>,
    handler: &ReinitializeHandler,
    state: ReinitializedState,
) -> (Apdu, AfterReply) {
    let mut data = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: state,
        password: None,
    }
    .encode(&mut data)
    .unwrap();
    let request = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 3,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::REINITIALIZE_DEVICE,
        service_request: data.freeze(),
    };
    let requester = Requester::new(&[10, 0, 0, 7], None, TransportProvenance::unverified());
    let mut accepted = false;
    let (reply, hooks) = response(
        db,
        &request,
        &None,
        Some(handler),
        requester,
        || Ok(()),
        |_, _| accepted = true,
    )
    .await;
    // The Audit record and the DCC transition follow acceptance, which a
    // refused activation never reaches (#1564).
    assert_eq!(accepted, matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    (reply, hooks)
}

fn assert_invalid_configuration(reply: &Apdu) {
    let Apdu::Error(error) = reply else {
        panic!("an Error PDU, not {reply:?}");
    };
    assert_eq!(error.error_class, ErrorClass::PROPERTY);
    assert_eq!(error.error_code, ErrorCode::INVALID_CONFIGURATION_DATA);
}

async fn changes_pending(db: &RwLock<ObjectDatabase>) -> bool {
    let value = db
        .read()
        .await
        .get(&port_oid())
        .unwrap()
        .read_property(P::CHANGES_PENDING, None)
        .unwrap();
    value == PropertyValue::Boolean(true)
}

/// The dry run refuses before the handler runs, so the handler never
/// schedules a restart for a request that fails.
#[tokio::test]
async fn a_refused_dry_run_answers_before_the_handler() {
    for state in [
        ReinitializedState::ACTIVATE_CHANGES,
        ReinitializedState::WARMSTART,
    ] {
        let tables = Arc::new(Tables::default());
        let (db, _lease) = database(&tables);
        tables.refuse_check.store(true, Ordering::SeqCst);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let handler: ReinitializeHandler = Arc::new(move |_, _| {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let (reply, hooks) = reinitialize(&db, &handler, state).await;
        assert_invalid_configuration(&reply);
        assert!(hooks.is_empty(), "{state:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "{state:?}");
        assert!(changes_pending(&db).await);
        assert_eq!(tables.activated.load(Ordering::SeqCst), 0);
    }
}

/// A refusal after the handler, a race with the transport, is answered
/// the same way, as an Error, and the values stay pending. The restart the
/// handler registered to run after the reply is dropped unrun (#1565).
#[tokio::test]
async fn a_late_activation_refusal_is_answered_the_same_way() {
    let tables = Arc::new(Tables::default());
    let (db, _lease) = database(&tables);
    let racing = Arc::clone(&tables);
    let ran = Arc::new(AtomicUsize::new(0));
    let restarts = Arc::clone(&ran);
    let handler: ReinitializeHandler = Arc::new(move |context, _| {
        let restarts = Arc::clone(&restarts);
        context.after_reply(move || {
            restarts.fetch_add(1, Ordering::SeqCst);
        });
        racing.refuse_activation.store(true, Ordering::SeqCst);
        Ok(())
    });
    for state in [
        ReinitializedState::ACTIVATE_CHANGES,
        ReinitializedState::WARMSTART,
    ] {
        let (reply, hooks) = reinitialize(&db, &handler, state).await;
        assert_invalid_configuration(&reply);
        assert!(hooks.is_empty(), "{state:?}");
        hooks.run();
        assert_eq!(ran.load(Ordering::SeqCst), 0, "{state:?}");
        assert!(changes_pending(&db).await);
        assert_eq!(tables.activated.load(Ordering::SeqCst), 0);
    }

    // Once the transport takes them again, the next request activates them.
    tables.refuse_activation.store(false, Ordering::SeqCst);
    let accepted = Arc::clone(&ran);
    let accept: ReinitializeHandler = Arc::new(move |context, _| {
        let accepted = Arc::clone(&accepted);
        context.after_reply(move || {
            accepted.fetch_add(1, Ordering::SeqCst);
        });
        Ok(())
    });
    let (reply, hooks) = reinitialize(&db, &accept, ReinitializedState::ACTIVATE_CHANGES).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert!(!changes_pending(&db).await);
    assert_eq!(tables.activated.load(Ordering::SeqCst), 1);
    assert_eq!(ran.load(Ordering::SeqCst), 0, "not before the reply");
    hooks.run();
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}
