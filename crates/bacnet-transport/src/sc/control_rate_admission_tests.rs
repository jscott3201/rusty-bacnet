//! The control-message rate limit at the SC NPDU hand-off (#518).
use super::*;
use crate::sc::control_rate::ScControlRateLimit;

/// A network-layer message: I-Am-Router-To-Network listing network 1000.
const I_AM_ROUTER: &[u8] = &[0x01, 0x80, 0x01, 0x03, 0xe8];
/// A confirmed-request APDU behind an ordinary NPDU header.
const APDU: &[u8] = &[0x01, 0x04, 0x00, 0x05, 0x01, 0x0c];

fn envelope(source: &MacAddr, npdu: &'static [u8]) -> ReceivedNpdu {
    ReceivedNpdu {
        direct_response: None,
        npdu: Bytes::from_static(npdu),
        source_mac: source.clone(),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

fn admission(burst: u32) -> ScNpduAdmission {
    ScNpduAdmission::new(ScNpduAdmissionPolicy {
        control_rate: ScControlRateLimit::new(burst, 1),
        ..Default::default()
    })
}

#[tokio::test(start_paused = true)]
async fn a_control_flood_is_capped_per_source_without_starving_apdus() {
    let admission = admission(4);
    let (tx, mut rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
    let flooder = MacAddr::from_slice(&[0x02; 6]);
    let other = MacAddr::from_slice(&[0x03; 6]);
    let mut admit = |source: &MacAddr, npdu, charge| {
        let outcome = admission.admit(
            &tx,
            ScIngressPath::Hub,
            source,
            envelope(source, npdu),
            charge,
        );
        // Drain as an application would, so the queue quota never binds.
        while rx.try_recv().is_ok() {}
        matches!(outcome, AdmissionOutcome::Admitted)
    };

    // The flooder's control messages stop at the burst...
    let control = (0..20)
        .filter(|_| admit(&flooder, I_AM_ROUTER, true))
        .count();
    assert_eq!(control, 4);
    // ...while its own APDUs, and another source's control messages, pass.
    assert_eq!((0..20).filter(|_| admit(&flooder, APDU, true)).count(), 20);
    assert_eq!(
        (0..4).filter(|_| admit(&other, I_AM_ROUTER, true)).count(),
        4
    );
    // An item another queue already charged is not charged again.
    assert!(admit(&flooder, I_AM_ROUTER, false));
    assert_eq!(
        admission.drop_counts(),
        ScNpduDropCounts {
            control_rate_drops: 16,
            ..Default::default()
        }
    );

    // The bucket refills at the configured rate.
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    let refilled = (0..5)
        .filter(|_| admit(&flooder, I_AM_ROUTER, true))
        .count();
    assert_eq!(refilled, 2);
}

#[test]
fn policy_validation_covers_the_control_rate() {
    let bad = ScNpduAdmissionPolicy {
        control_rate: ScControlRateLimit::new(0, 1),
        ..Default::default()
    };
    assert!(bad.validate().is_err());
    assert!(ScNpduAdmissionPolicy::default().validate().is_ok());
}

#[test]
fn a_direct_certificate_keeps_one_budget_across_vmacs() {
    let admission = admission(4);
    let (tx, mut rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
    let identity = |incarnation| {
        TransportProvenance::verified_direct_peer(
            crate::port::DirectScIdentity::verified([7; 32], incarnation),
            Default::default(),
        )
    };
    let mut admit = |vmac: u8, incarnation| {
        let source = MacAddr::from_slice(&[vmac; 6]);
        let mut item = envelope(&source, I_AM_ROUTER);
        item.provenance = identity(incarnation);
        let outcome = admission.admit(&tx, ScIngressPath::Direct, &source, item, true);
        while rx.try_recv().is_ok() {}
        matches!(outcome, AdmissionOutcome::Admitted)
    };
    // Two of the burst on one VMAC, then a reconnect on another VMAC: the
    // same certificate has only the other two left.
    assert_eq!((0..2).filter(|_| admit(0x10, 1)).count(), 2);
    assert_eq!((0..5).filter(|_| admit(0x11, 2)).count(), 2);
    // An unverified source on that same VMAC has its own budget.
    let source = MacAddr::from_slice(&[0x11; 6]);
    let outcome = admission.admit(
        &tx,
        ScIngressPath::Direct,
        &source,
        envelope(&source, I_AM_ROUTER),
        true,
    );
    assert!(matches!(outcome, AdmissionOutcome::Admitted));
}
