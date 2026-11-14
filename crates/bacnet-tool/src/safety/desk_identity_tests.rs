//! A change goes to its address only while the device there is still the
//! one it was planned and confirmed for (#2560): the discovery table's row
//! at the address says so, and a revert to an address with no row asks the
//! device first. An entry kept without an address merges with a later
//! write only where its instance is, and a revert that may not look for a
//! device sends no Who-Is.

use bacnet_services::read_property::ReadPropertyRequest;

use super::*;
use crate::safety::{Unsent, turn};

/// A caller that may not look for a device, as MCP's caller.
struct Agent;

impl Caller for Agent {
    fn level(&self) -> Level {
        Level::Armed { dangerous: false }
    }
    fn refusal(&self) -> Option<String> {
        None
    }
    fn may_look(&self) -> bool {
        false
    }
}

/// [`LEDGER_BEFORE_ADDRESSES`]'s entry as this release writes it, at
/// `address`.
fn ledger_at(address: &str) -> String {
    let with = format!("\"device\": 7,\n      \"address\": \"{address}\",");
    LEDGER_BEFORE_ADDRESSES.replace("\"device\": 7,", &with)
}

/// A workspace holding `ledger`.
fn workspace(ledger: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(crate::safety::ledger::FILE), ledger).unwrap();
    dir
}

/// P1: the write is planned for device 7 at A; before it is sent, A
/// announces itself as device 9. The turn is refused, naming the address
/// and both instances, and the write is not sent: A is read twice (the
/// plan and the re-read), never written.
#[tokio::test(start_paused = true)]
async fn a_write_is_not_sent_once_its_address_is_another_device() {
    let lab = lab().await;
    lab.client.add_device(DEVICE, &A).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    let specs = [spec(at(&A))];
    let preview = desk
        .prepare(&lab.client, &lab.rate, &Armed, &specs, false, false)
        .await
        .unwrap();

    lab.client.add_device(9, &A).await.unwrap();
    let refused = turn(&lab.client, &lab.rate, &at(&A), DEVICE).await.err();
    assert!(
        matches!(
            &refused,
            Some(Unsent::OtherDevice { address, expected: DEVICE, found: 9, .. }) if *address == at(&A)
        ),
        "{refused:?}"
    );
    let done = desk
        .execute(&lab.client, &lab.rate, &Armed, &preview, false)
        .await;
    let why = "10.0.0.2:47808 is device 9 now, not device 7";
    assert!(
        matches!(&done[0].outcome, Outcome::NotSent(text) if text.starts_with(why)),
        "{done:?}"
    );
    let rpm = ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE;
    assert_eq!(
        requests(&lab.sent),
        [(rpm, A.to_vec()), (rpm, A.to_vec())],
        "read, never written"
    );
    assert!(desk.entries().is_empty(), "settled as not applied");
}

/// P2: a write to device 7 at A is held in the ledger. Then A is device 9
/// and device 7 answers at B. The revert is refused with nothing sent, to A
/// or to B, and the entry stays for a later revert.
#[tokio::test(start_paused = true)]
async fn a_revert_is_not_sent_once_its_address_is_another_device() {
    let lab = lab().await;
    lab.client.add_device(DEVICE, &A).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    let specs = [spec(at(&A))];
    let preview = desk
        .prepare(&lab.client, &lab.rate, &Armed, &specs, false, false)
        .await
        .unwrap();
    let done = desk.commit(&lab.client, &lab.rate, &Armed, &preview).await;
    assert_eq!(done[0].outcome, Outcome::Ack);

    lab.client.add_device(9, &A).await.unwrap();
    lab.client.add_device(DEVICE, &B).await.unwrap();
    lab.sent.clear();
    let reverted = desk.revert(&lab.client, &lab.rate, &Armed, |_| true).await;
    let why = "10.0.0.2:47808 is device 9 now, not device 7";
    assert!(
        matches!(&reverted[0].1, Outcome::NotSent(text) if text.starts_with(why)),
        "{reverted:?}"
    );
    assert!(lab.sent.is_empty(), "nothing to A, nor to B");
    assert_eq!(desk.entries().len(), 1, "kept for a later revert");
}

/// After a restart the table has no row at the entry's address A. The
/// revert asks the device there which it is first: one ReadProperty of
/// the Device object's Object_Identifier by the wildcard instance, and no
/// Who-Is. Device 7 answering is reverted; device 9 answering, or no
/// answer, is refused with nothing written, and the entry stays.
#[tokio::test(start_paused = true)]
async fn a_revert_with_no_row_asks_the_device_which_it_is() {
    let rp = ConfirmedServiceChoice::READ_PROPERTY;
    let wp = ConfirmedServiceChoice::WRITE_PROPERTY;
    let wildcard =
        ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE).unwrap();
    for identity in [Some(DEVICE), Some(9), None] {
        let dir = workspace(&ledger_at("10.0.0.2:47808"));
        let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
        let lab = lab_as(identity).await;
        let done = desk.revert(&lab.client, &lab.rate, &Armed, |_| true).await;

        assert!(lab.sent.broadcasts().is_empty(), "{identity:?}: no Who-Is");
        let frames = lab.sent.unicasts();
        let Apdu::ConfirmedRequest(asked) = frames[0].apdu() else {
            panic!("{frames:?}");
        };
        let read = ReadPropertyRequest::decode(&asked.service_request).unwrap();
        assert_eq!(asked.service_choice, rp);
        assert_eq!(read.object_identifier, wildcard);
        assert_eq!(
            read.property_identifier,
            PropertyIdentifier::OBJECT_IDENTIFIER
        );
        if identity == Some(DEVICE) {
            assert_eq!(done[0].1, Outcome::Ack, "{done:?}");
            assert_eq!(requests(&lab.sent), [(rp, A.to_vec()), (wp, A.to_vec())]);
            assert!(desk.entries().is_empty(), "undone");
        } else {
            assert!(matches!(done[0].1, Outcome::NotSent(_)), "{done:?}");
            if identity.is_some() {
                let why = "10.0.0.2:47808 is device 9 now, not device 7";
                assert!(
                    matches!(&done[0].1, Outcome::NotSent(text) if text.starts_with(why)),
                    "{done:?}"
                );
            }
            assert_eq!(requests(&lab.sent), [(rp, A.to_vec())], "nothing written");
            assert_eq!(desk.entries().len(), 1, "kept");
        }
    }
}

/// An entry from before addresses (device 7, slot 12 of AO:1) and a new
/// write to the same slot at A. With device 7 at A alone, the entry is the
/// device at A: the write merges into it, its first pre-image stays and it
/// keeps A. With B claiming 7 too, which device it changed is unknown: the
/// two are kept apart, and a revert of the old one is refused, saying what
/// the operator can do.
#[tokio::test(start_paused = true)]
async fn an_entry_from_before_addresses_merges_only_where_its_instance_is() {
    for contested in [false, true] {
        let dir = workspace(LEDGER_BEFORE_ADDRESSES);
        let lab = lab().await;
        lab.client.add_device(DEVICE, &A).await.unwrap();
        if contested {
            lab.client.add_device(DEVICE, &B).await.unwrap();
        }
        let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
        let specs = [spec(at(&A))];
        let preview = desk
            .prepare(&lab.client, &lab.rate, &Armed, &specs, false, false)
            .await
            .unwrap();
        let done = desk.commit(&lab.client, &lab.rate, &Armed, &preview).await;
        assert_eq!(done[0].outcome, Outcome::Ack, "{done:?}");
        let kept = desk.entries();
        if !contested {
            let [entry] = kept.as_slice() else {
                panic!("{kept:?}");
            };
            assert_eq!((entry.id, entry.address.as_ref()), (1, Some(&at(&A))));
            assert_eq!(entry.before, None, "the first pre-image");
            continue;
        }
        let [old, new] = kept.as_slice() else {
            panic!("{kept:?}");
        };
        assert_eq!((old.id, old.address.as_ref()), (1, None), "kept apart");
        assert_eq!(new.address.as_ref(), Some(&at(&A)));

        lab.sent.clear();
        let reverted = desk
            .revert(&lab.client, &lab.rate, &Armed, |e| e.address.is_none())
            .await;
        let why = "revert it again once one address alone claims 7";
        assert!(
            matches!(&reverted[0].1, Outcome::NotSent(text) if text.contains(why)),
            "{reverted:?}"
        );
        assert!(lab.sent.is_empty(), "nothing sent");
    }
}

/// A caller that may not look (MCP's) sends no Who-Is: a routed entry
/// whose address has no row, and an entry from before addresses whose
/// instance no row claims, are reported not reverted, saying why, with
/// nothing sent. The caller that may look sends one Who-Is for each.
#[tokio::test(start_paused = true)]
async fn a_revert_that_may_not_look_sends_no_who_is() {
    let routed = ledger_at("2:0x05");
    for ledger in [routed.as_str(), LEDGER_BEFORE_ADDRESSES] {
        let dir = workspace(ledger);
        let lab = lab().await;
        let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
        let done = desk.revert(&lab.client, &lab.rate, &Agent, |_| true).await;
        let why = "this revert sends no Who-Is";
        assert!(
            matches!(&done[0].1, Outcome::NotSent(text) if text.contains(why)),
            "{done:?}"
        );
        assert!(lab.sent.is_empty(), "nothing sent");
        assert_eq!(desk.entries().len(), 1, "kept");

        let done = desk.revert(&lab.client, &lab.rate, &Armed, |_| true).await;
        assert!(matches!(done[0].1, Outcome::NotSent(_)), "{done:?}");
        assert_eq!(lab.sent.broadcasts().len(), 1, "the TUI looks");
    }
}
