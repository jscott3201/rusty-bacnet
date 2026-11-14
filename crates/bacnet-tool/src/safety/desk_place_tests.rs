//! Where a change goes (#2560) when the table has no row at its address,
//! when the row changes while the change waits for its turn, when the
//! caller refuses the address a revert found, and when the ledger holds an
//! entry from before addresses beside one at the address: against the
//! scripted device, which keeps AO:1's priority slots, so each test can
//! say how the slot ends.

use bacnet_types::MacAddr;
use bytes::Bytes;
use serde_json::{Value, json};

use super::*;
use crate::safety::ledger::FILE;

const NULL: &[u8] = &[0x00];
const FOUR: &[u8] = &[0x44, 0x40, 0x80, 0, 0];
const FIVE: &[u8] = &[0x44, 0x40, 0xA0, 0, 0];
const SIX: &[u8] = &[0x44, 0x40, 0xC0, 0, 0];

/// Present_Value `value` at priority 12 on AO:1 of device 7 at A.
fn writing(value: &str) -> WriteSpec {
    WriteSpec {
        value: Some(value.into()),
        ..spec(at(&A))
    }
}

/// Plan and send one write of `value` (the desk's read, then its commit).
async fn write(desk: &mut Desk, lab: &Lab, value: &str) -> Outcome {
    let specs = [writing(value)];
    let preview = desk
        .prepare(&lab.client, &lab.rate, &Armed, &specs, false, false)
        .await
        .unwrap();
    let mut done = desk.commit(&lab.client, &lab.rate, &Armed, &preview).await;
    done.remove(0).outcome
}

/// The WriteProperty requests sent: where each went, its priority and the
/// value it carried.
fn writes(sent: &SendLog) -> Vec<(Vec<u8>, Option<u8>, Vec<u8>)> {
    sent.unicasts()
        .iter()
        .filter_map(|frame| match frame.apdu() {
            Apdu::ConfirmedRequest(request)
                if request.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY =>
            {
                let write = WritePropertyRequest::decode(&request.service_request).unwrap();
                Some((frame.mac.to_vec(), write.priority, write.property_value))
            }
            _ => None,
        })
        .collect()
}

/// Slot 12 of AO:1 on the scripted device at `mac`.
fn slot12(lab: &Lab, mac: &[u8]) -> Vec<u8> {
    slots_at(&lab.slots, mac)[11].clone()
}

/// Slot 12 at `mac` holding `value`, as an earlier write left it.
fn hold(lab: &Lab, mac: &[u8], value: &[u8]) {
    let mut slots = slots_at(&lab.slots, mac);
    slots[11] = value.to_vec();
    lab.slots.lock().unwrap().insert(mac.to_vec(), slots);
}

/// A workspace whose ledger holds `entries`.
fn workspace(entries: &[Value]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let file = json!({ "version": 1, "entries": entries });
    std::fs::write(dir.path().join(FILE), file.to_string()).unwrap();
    dir
}

/// [`LEDGER_BEFORE_ADDRESSES`]'s entry (slot 12 was empty, 5 written).
fn old_entry() -> Value {
    let file: Value = serde_json::from_str(LEDGER_BEFORE_ADDRESSES).unwrap();
    file["entries"][0].clone()
}

/// An entry of this release's: slot `priority` at `address`, `before`
/// there, `written` now.
fn entry_at(id: u64, address: &str, priority: u8, (before, written): (&str, &str)) -> Value {
    let mut entry = old_entry();
    entry["id"] = json!(id);
    entry["seq"] = json!(id);
    entry["address"] = json!(address);
    entry["priority"] = json!(priority);
    entry["before"] = json!({ "bytes": before, "text": before });
    entry["written"] = json!({ "bytes": written, "text": written });
    entry
}

/// Teach the client its network's number, as a router's
/// Network-Number-Is does.
async fn learn(lab: &Lab, network: u16) {
    let [hi, lo] = network.to_be_bytes();
    let says = Bytes::from(vec![1, 0x80, 0x13, hi, lo, 1]);
    let from = MacAddr::from_slice(&B);
    let frame = ReceivedNpdu::unverified(says, from, true, Vec::new(), None);
    lab.inbox.send(frame).await.unwrap();
    for _ in 0..1_000 {
        if lab.client.network_number() == Some(network) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("never learned network {network}");
}

/// Revert every entry; true when each was acknowledged.
async fn revert_all(desk: &mut Desk, lab: &Lab) -> Vec<(Entry, Outcome)> {
    desk.revert(&lab.client, &lab.rate, &Armed, |_| true).await
}

/// The reviewer's sequence: an entry from before addresses (E0: slot 12
/// was empty, 5 written) while device 7 is claimed at A and at B; a write
/// of 4 at A makes its own entry (E1, 5 before it); B announces another
/// device; a write of 6 at A goes on with E1, the entry at its address,
/// not with E0. Reverting all, newest first, puts 5 back from E1, then
/// empties the slot from E0: the slot ends as it began, nothing goes to B,
/// and the ledger is empty.
#[tokio::test(start_paused = true)]
async fn revert_all_after_an_old_entry_and_a_new_one_restores_the_original() {
    let dir = workspace(&[old_entry()]);
    let lab = lab().await;
    hold(&lab, &A, FIVE);
    lab.client.add_device(DEVICE, &A).await.unwrap();
    lab.client.add_device(DEVICE, &B).await.unwrap();
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    assert_eq!(write(&mut desk, &lab, "4").await, Outcome::Ack);
    lab.client.add_device(99, &B).await.unwrap();
    assert_eq!(write(&mut desk, &lab, "6").await, Outcome::Ack);
    assert_eq!(slot12(&lab, &A), SIX);
    let kept = desk.entries();
    let [old, new] = kept.as_slice() else {
        panic!("{kept:?}");
    };
    assert_eq!((old.address.as_ref(), &old.written.bytes[..]), (None, FIVE));
    let before = new.before.as_ref().map(|b| &b.bytes[..]);
    assert_eq!(
        (new.address.as_ref(), before, &new.written.bytes[..]),
        (Some(&at(&A)), Some(FIVE), SIX)
    );

    lab.sent.clear();
    let done = revert_all(&mut desk, &lab).await;
    assert!(done.iter().all(|(_, o)| *o == Outcome::Ack), "{done:?}");
    assert_eq!(
        writes(&lab.sent),
        [
            (A.to_vec(), Some(12), FIVE.to_vec()),
            (A.to_vec(), Some(12), NULL.to_vec())
        ]
    );
    assert_eq!(slot12(&lab, &A), NULL, "the slot as it began");
    assert!(desk.entries().is_empty());
}

/// An entry kept in routed form on the client's own network (written
/// before the client learned the number) is the local station's entry.
/// Once the number is known and device 7 is at A alone, a write of 6 there
/// goes on with that entry, not with the entry from before addresses, so
/// the two never hold one target with their pre-images out of order:
/// reverting all ends with the slot as it began.
#[tokio::test(start_paused = true)]
async fn an_entry_in_routed_form_on_this_network_takes_the_write_at_its_station() {
    let routed = "7:0x0A000002BAC0";
    let dir = workspace(&[
        old_entry(),
        entry_at(2, routed, 12, ("4440a00000", "4440800000")),
    ]);
    let lab = lab_with(Some(DEVICE), true).await;
    learn(&lab, 7).await;
    hold(&lab, &A, FOUR);
    lab.client.add_device(DEVICE, &A).await.unwrap();
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    assert_eq!(write(&mut desk, &lab, "6").await, Outcome::Ack);
    let kept = desk.entries();
    let [old, new] = kept.as_slice() else {
        panic!("{kept:?}");
    };
    assert_eq!((old.id, old.address.as_ref()), (1, None), "E0 untouched");
    assert_eq!((new.id, new.address.as_ref()), (2, Some(&at(&A))));
    assert_eq!(&new.written.bytes[..], SIX);

    lab.sent.clear();
    let done = revert_all(&mut desk, &lab).await;
    assert!(done.iter().all(|(_, o)| *o == Outcome::Ack), "{done:?}");
    assert_eq!(
        writes(&lab.sent),
        [
            (A.to_vec(), Some(12), FIVE.to_vec()),
            (A.to_vec(), Some(12), NULL.to_vec())
        ]
    );
    assert_eq!(slot12(&lab, &A), NULL, "the slot as it began");
}

/// A local address with no row (aged out of the table) is asked which
/// device it is before a write goes there, as a revert's is: device 7
/// answering is written; device 9, or no answer, is refused with nothing
/// written, saying what was asked. No Who-Is either way.
#[tokio::test(start_paused = true)]
async fn a_write_to_a_local_address_with_no_row_asks_the_device_first() {
    let (rpm, rp, wp) = (
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        ConfirmedServiceChoice::READ_PROPERTY,
        ConfirmedServiceChoice::WRITE_PROPERTY,
    );
    for identity in [Some(DEVICE), Some(9), None] {
        let lab = lab_as(identity).await;
        let dir = tempfile::tempdir().unwrap();
        let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
        let outcome = write(&mut desk, &lab, "5").await;
        assert!(lab.sent.broadcasts().is_empty(), "{identity:?}: no Who-Is");
        let asked = [(rpm, A.to_vec()), (rp, A.to_vec())];
        let text = match &outcome {
            Outcome::NotSent(text) => text.as_str(),
            _ => "",
        };
        match identity {
            Some(DEVICE) => {
                assert_eq!(outcome, Outcome::Ack);
                let mut sent = asked.to_vec();
                sent.push((wp, A.to_vec()));
                assert_eq!(requests(&lab.sent), sent);
            }
            Some(_) => {
                assert!(
                    text.starts_with("10.0.0.2:47808 is device 9 now"),
                    "{outcome:?}"
                );
                assert_eq!(requests(&lab.sent), asked, "nothing written");
            }
            None => {
                assert!(
                    text.starts_with("asked 10.0.0.2:47808 which device it is")
                        && text.contains("nothing was sent there")
                        && text.contains("discover the device"),
                    "{outcome:?}"
                );
                assert_eq!(requests(&lab.sent), asked, "nothing written");
            }
        }
    }
}

/// The row is checked again once the turn is held: the address
/// re-announces as device 9 while the write waits for its gate, and the
/// write is refused with nothing sent.
#[tokio::test(start_paused = true)]
async fn a_row_that_changes_during_the_wait_refuses_the_write() {
    let lab = lab().await;
    lab.client.add_device(DEVICE, &A).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    let specs = [spec(at(&A))];
    let preview = desk
        .prepare(&lab.client, &lab.rate, &Armed, &specs, false, false)
        .await
        .unwrap();
    let held = lab.rate.turn(&at(&A)).await;
    let client = &lab.client;
    let release = async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        client.add_device(9, &A).await.unwrap();
        drop(held);
    };
    let (done, ()) = tokio::join!(
        desk.commit(&lab.client, &lab.rate, &Armed, &preview),
        release
    );
    let why = "10.0.0.2:47808 is device 9 now, not device 7";
    assert!(
        matches!(&done[0].outcome, Outcome::NotSent(text) if text.starts_with(why)),
        "{done:?}"
    );
    let rpm = ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE;
    assert_eq!(
        requests(&lab.sent),
        [(rpm, A.to_vec())],
        "only the plan's read"
    );
    assert!(desk.entries().is_empty());
}

/// A revert of several entries at one address with no row asks the device
/// there which it is once: both slots are put back after one ReadProperty,
/// and with no answer both are refused after that one.
#[tokio::test(start_paused = true)]
async fn a_revert_asks_each_address_with_no_row_once() {
    let (rp, wp) = (
        ConfirmedServiceChoice::READ_PROPERTY,
        ConfirmedServiceChoice::WRITE_PROPERTY,
    );
    let entries = [
        entry_at(1, "10.0.0.2:47808", 12, ("00", "4440a00000")),
        entry_at(2, "10.0.0.2:47808", 13, ("00", "4440a00000")),
    ];
    for identity in [Some(DEVICE), None] {
        let dir = workspace(&entries);
        let lab = lab_as(identity).await;
        let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
        let done = revert_all(&mut desk, &lab).await;
        if identity.is_some() {
            assert!(done.iter().all(|(_, o)| *o == Outcome::Ack), "{done:?}");
            let sent = [(rp, A.to_vec()), (wp, A.to_vec()), (wp, A.to_vec())];
            assert_eq!(requests(&lab.sent), sent);
        } else {
            assert!(done.iter().all(|(_, o)| matches!(o, Outcome::NotSent(_))));
            assert_eq!(requests(&lab.sent), [(rp, A.to_vec())], "asked once");
        }
    }
}

/// A caller that refuses where a revert would go.
struct Refusing;

impl Caller for Refusing {
    fn level(&self) -> Level {
        Level::Armed { dangerous: false }
    }
    fn refusal(&self) -> Option<String> {
        None
    }
    fn may_look(&self) -> bool {
        true
    }
    fn revert_refusal(&self, entry: &Entry, at: &DeviceAddress) -> Option<String> {
        Some(format!("not device {} at {at}", entry.device))
    }
}

/// The caller sees where the desk found a revert's device before anything
/// goes there, the identity read included, and may refuse it: the entry is
/// not reverted, says why, and stays.
#[tokio::test(start_paused = true)]
async fn a_caller_refuses_a_revert_before_anything_is_sent() {
    let dir = workspace(&[entry_at(1, "10.0.0.2:47808", 12, ("00", "4440a00000"))]);
    let lab = lab().await;
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    let done = desk
        .revert(&lab.client, &lab.rate, &Refusing, |_| true)
        .await;
    assert_eq!(
        done[0].1,
        Outcome::NotSent("not device 7 at 10.0.0.2:47808".into())
    );
    assert!(lab.sent.is_empty(), "no identity read, no write");
    assert_eq!(desk.entries().len(), 1);
}
