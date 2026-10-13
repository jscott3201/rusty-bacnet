//! This node's own management exchanges: admission by peer and operation,
//! matching a response with no invoke ID, generations, early, late and
//! duplicate responses, and timeouts (Annex J.2.1).

mod node_support;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry, BvlcResultCode, Bvll, FdtEntries};
use bacnet_bip_core::node::{
    Action, ExchangeId, ExchangeRefusal, Inbound, LocalFailure, Mode, Node, Operation, Permissive,
    Policy,
};
use bacnet_bip_core::{Duration, Instant};
use node_support::*;

const TIMEOUT: Duration = Duration::from_secs(3);

fn node() -> Node {
    Node::new(link(Mode::Normal), Instant::ZERO).unwrap()
}

/// Begin and send `request` to `peer` at `now`.
fn sent<const X: usize>(
    node: &mut Node<X>,
    peer: SocketAddrV4,
    request: Bvll<'_>,
    now: Instant,
) -> ExchangeId {
    let (exchange, outgoing) = node.begin_exchange(peer, request, TIMEOUT, now).unwrap();
    assert_eq!((outgoing.to, outgoing.message), (peer, request));
    assert!(!node.exchange_sent(exchange, now));
    exchange
}

/// What `node` makes of `payload` from `peer`, unicast to it.
fn answer<'d, const X: usize>(
    node: &mut Node<X>,
    payload: &'d [u8],
    peer: SocketAddrV4,
    now: Instant,
) -> Inbound<'d> {
    node.receive(&datagram(payload, peer, UNICAST), now, &mut Permissive)
}

fn read_bdt() -> Bvll<'static> {
    Bvll::ReadBroadcastDistributionTable
}

fn write_bdt() -> Bvll<'static> {
    Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(&[]))
}

fn register() -> Bvll<'static> {
    Bvll::RegisterForeignDevice { ttl: 60 }
}

/// Whether `inbound` settles `exchange` now with an accepted answer.
fn settled(inbound: &Inbound<'_>, exchange: ExchangeId) -> bool {
    matches!(
        inbound,
        Inbound::Matched { exchange: e, message: Ok(_), accepted: true, settled: true }
            if *e == exchange
    )
}

#[test]
fn a_result_is_matched_and_never_answered() {
    let mut node = node();
    let exchange = sent(&mut node, PEER, write_bdt(), at(0));
    let success = result(SUCCESS);
    let inbound = answer(&mut node, &success, PEER, at(10));
    assert!(settled(&inbound, exchange));
    assert!(drain(&mut node).is_empty());
    // With nothing waiting, a result is only counted.
    let stray = result(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);
    assert_eq!(
        answer(&mut node, &stray, BBMD, at(20)),
        Inbound::Unmatched {
            peer: BBMD,
            code: Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK),
            late: false
        }
    );
    let counters = node.take_counters();
    assert_eq!(counters.write_bdt.results, 1);
    assert_eq!(counters.write_bdt.last_result, Some(SUCCESS));
    assert_eq!(counters.unmatched_responses, 1);
    assert_eq!(
        counters.last_unmatched_result,
        Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK)
    );
}

#[test]
fn each_operation_is_matched_by_its_own_answers() {
    let rows = [BdtEntry::new(PEER, Ipv4Addr::BROADCAST)];
    let bdt_ack = wire(Bvll::ReadBroadcastDistributionTableAck(
        BdtEntries::from_entries(&rows),
    ));
    let fdt_ack = wire(Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(
        &[],
    )));
    let requests = [
        (Operation::ReadBdt, read_bdt()),
        (Operation::WriteBdt, write_bdt()),
        (Operation::ReadFdt, Bvll::ReadForeignDeviceTable),
        (
            Operation::DeleteFdtEntry,
            Bvll::DeleteForeignDeviceTableEntry(REMOTE),
        ),
        (Operation::RegisterForeignDevice, register()),
    ];
    for (operation, request) in requests {
        let own_nak = result(operation.function().nak().unwrap());
        let mut answers = vec![
            (own_nak, true),
            (result(SUCCESS), operation.answered_by_success()),
        ];
        answers.push((bdt_ack.clone(), operation == Operation::ReadBdt));
        answers.push((fdt_ack.clone(), operation == Operation::ReadFdt));
        // Another operation's NAK, and an undefined code, answer nothing.
        let other = Operation::ALL
            .into_iter()
            .find(|other| *other != operation)
            .unwrap();
        answers.push((result(other.function().nak().unwrap()), false));
        answers.push((
            wire(Bvll::BvlcResult(
                BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
            )),
            false,
        ));
        for (payload, fits) in answers {
            let mut node = node();
            let exchange = sent(&mut node, PEER, request, at(0));
            let inbound = answer(&mut node, &payload, PEER, at(5));
            assert_eq!(
                settled(&inbound, exchange),
                fits,
                "{operation:?} {payload:02X?}"
            );
            assert_eq!(matches!(inbound, Inbound::Unmatched { .. }), !fits);
        }
    }
}

#[test]
fn exchanges_are_keyed_by_peer_and_operation() {
    let mut node: Node<4> = node();
    let other_peer = addr(192, 168, 1, 30, PORT);
    sent(&mut node, PEER, read_bdt(), at(0));
    sent(&mut node, other_peer, read_bdt(), at(0));
    let begin = |node: &mut Node<4>, peer, request| {
        node.begin_exchange(peer, request, TIMEOUT, at(1))
            .map(|_| ())
    };
    assert_eq!(
        begin(&mut node, PEER, read_bdt()),
        Err(ExchangeRefusal::SameExchange)
    );
    sent(&mut node, PEER, write_bdt(), at(1));
    // A success can't tell Write-BDT from Register-Foreign-Device.
    assert_eq!(
        begin(&mut node, PEER, register()),
        Err(ExchangeRefusal::ResultClass)
    );
    assert_eq!(
        begin(&mut node, PEER, Bvll::DeleteForeignDeviceTableEntry(REMOTE)),
        Err(ExchangeRefusal::ResultClass)
    );
    sent(&mut node, PEER, Bvll::ReadForeignDeviceTable, at(1));
    // Four slots, four live.
    assert_eq!(
        begin(&mut node, other_peer, register()),
        Err(ExchangeRefusal::Full)
    );
    assert_eq!(
        begin(&mut node, PEER, Bvll::OriginalUnicastNpdu(&[1, 0])),
        Err(ExchangeRefusal::NotARequest)
    );
    let counters = node.take_counters();
    assert_eq!(counters.read_bdt.busy, 1);
    assert_eq!(counters.register_foreign_device.busy, 2);
    assert_eq!(counters.delete_fdt_entry.busy, 1);
    // The peer's address and port both name it.
    let elsewhere = addr(192, 168, 1, 20, 47_809);
    assert!(matches!(
        answer(&mut node, &result(SUCCESS), elsewhere, at(2)),
        Inbound::Unmatched { .. }
    ));
}

#[test]
fn a_stale_handle_changes_nothing() {
    let mut node: Node<1> = Node::new(link(Mode::Normal), Instant::ZERO).unwrap();
    let old = sent(&mut node, PEER, read_bdt(), at(0));
    node.exchange_failed(old, LocalFailure::Cancelled, at(1));
    let (new, _) = node
        .begin_exchange(PEER, read_bdt(), TIMEOUT, at(2))
        .unwrap();
    assert_eq!(new.slot(), old.slot());
    assert_ne!(new.generation(), old.generation());
    // The old handle neither sends nor ends the new exchange.
    assert!(!node.exchange_sent(old, at(2)));
    node.exchange_failed(old, LocalFailure::SendFailed, at(2));
    assert!(!node.exchange_sent(new, at(2)) && node.next_deadline() == Some(at(3_002)));
    let nak = result(BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK);
    assert!(settled(&answer(&mut node, &nak, PEER, at(3)), new));
    // Reported twice, the send counts once.
    assert!(!node.exchange_sent(new, at(4)));
    let counters = node.take_counters();
    assert_eq!(
        (counters.read_bdt.sent, counters.read_bdt.local_errors),
        (2, 0)
    );
}

#[test]
fn a_response_that_beats_the_send_settles_it() {
    let mut node = node();
    let (exchange, _) = node
        .begin_exchange(PEER, write_bdt(), TIMEOUT, at(0))
        .unwrap();
    let first = result(SUCCESS);
    assert_eq!(
        answer(&mut node, &first, PEER, at(1)),
        Inbound::Matched {
            exchange,
            message: Ok(Bvll::BvlcResult(SUCCESS)),
            accepted: true,
            settled: false
        }
    );
    let second = result(BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK);
    assert_eq!(
        answer(&mut node, &second, PEER, at(1)),
        Inbound::Duplicate { exchange }
    );
    assert_eq!(node.next_deadline(), None);
    assert!(node.exchange_sent(exchange, at(2)));
    let counters = node.take_counters();
    assert_eq!(
        (counters.write_bdt.sent, counters.write_bdt.results),
        (1, 1)
    );
    assert_eq!(counters.write_bdt.last_result, Some(SUCCESS));
    assert_eq!(node.next_deadline(), None);

    // A failed send drops the early response, uncounted.
    let (exchange, _) = node
        .begin_exchange(PEER, write_bdt(), TIMEOUT, at(3))
        .unwrap();
    assert!(matches!(
        answer(&mut node, &first, PEER, at(4)),
        Inbound::Matched { settled: false, .. }
    ));
    node.exchange_failed(exchange, LocalFailure::SendFailed, at(5));
    let counters = node.take_counters();
    assert_eq!(
        (counters.write_bdt.results, counters.write_bdt.local_errors),
        (0, 1)
    );
    assert!(!node.exchange_sent(exchange, at(6)));

    // A refused table that beats the send says so too, and settles it
    // unanswered at the send.
    let (exchange, _) = node
        .begin_exchange(PEER, read_bdt(), TIMEOUT, at(7))
        .unwrap();
    let rows = [
        BdtEntry::new(PEER, Ipv4Addr::BROADCAST),
        BdtEntry::new(REMOTE, Ipv4Addr::BROADCAST),
    ];
    let big = wire(Bvll::ReadBroadcastDistributionTableAck(
        BdtEntries::from_entries(&rows),
    ));
    assert!(matches!(
        node.receive(&datagram(&big, PEER, UNICAST), at(8), &mut OneRow),
        Inbound::Matched {
            accepted: false,
            settled: false,
            ..
        }
    ));
    assert!(node.exchange_sent(exchange, at(9)));
    assert_eq!(node.take_counters().read_bdt.acknowledgements, 0);
}

#[test]
fn a_manual_exchange_times_out_at_its_deadline() {
    let mut node = node();
    let exchange = sent(&mut node, PEER, read_bdt(), at(1_000));
    assert_eq!(node.next_deadline(), Some(at(4_000)));
    node.on_deadline(at(3_999));
    assert!(drain(&mut node).is_empty());
    node.on_deadline(at(4_000));
    assert_eq!(drain(&mut node), [Action::ExchangeEnded { exchange }]);
    assert_eq!(node.next_deadline(), None);
    assert_eq!(node.take_counters().read_bdt.timeouts, 1);
}

#[test]
fn a_result_at_the_deadline_is_late_in_either_order() {
    let nak = result(BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK);
    for timer_first in [false, true] {
        let mut node = node();
        let exchange = sent(&mut node, PEER, read_bdt(), at(0));
        if timer_first {
            node.on_deadline(at(3_000));
        }
        assert_eq!(
            answer(&mut node, &nak, PEER, at(3_000)),
            Inbound::Unmatched {
                peer: PEER,
                code: Some(BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK),
                late: true
            }
        );
        assert_eq!(drain(&mut node), [Action::ExchangeEnded { exchange }]);
        let counters = node.take_counters();
        assert_eq!(
            (counters.read_bdt.timeouts, counters.late_responses),
            (1, 1)
        );
    }
}

#[test]
fn late_and_duplicate_results_are_told_apart() {
    let mut node = node();
    let first = sent(&mut node, PEER, register(), at(0));
    assert!(settled(
        &answer(&mut node, &result(SUCCESS), PEER, at(10)),
        first
    ));
    // A copy within one response timeout of the end is late; after, not.
    let late = Inbound::Unmatched {
        peer: PEER,
        code: Some(SUCCESS),
        late: true,
    };
    assert_eq!(answer(&mut node, &result(SUCCESS), PEER, at(3_009)), late);
    assert_eq!(
        answer(&mut node, &result(SUCCESS), PEER, at(3_010)),
        Inbound::Unmatched {
            peer: PEER,
            code: Some(SUCCESS),
            late: false
        }
    );
    // A late result while a newer exchange of its key is live goes to it.
    let second = sent(&mut node, PEER, register(), at(4_000));
    node.on_deadline(at(7_000));
    drain(&mut node);
    let third = sent(&mut node, PEER, register(), at(8_000));
    assert!(settled(
        &answer(&mut node, &result(SUCCESS), PEER, at(8_001)),
        third
    ));
    assert_ne!(second, third);
    let counters = node.take_counters();
    assert_eq!(counters.registration_attempts, 3);
    assert_eq!(counters.registrations_accepted, 2);
    assert_eq!(counters.register_foreign_device.timeouts, 1);
    assert_eq!(
        (counters.unmatched_responses, counters.late_responses),
        (2, 1)
    );
}

/// A policy whose table limit is one row.
struct OneRow;

impl Policy for OneRow {
    fn accept_table(&mut self, ack: &Bvll<'_>) -> bool {
        match ack {
            Bvll::ReadBroadcastDistributionTableAck(table) => table.len() <= 1,
            _ => true,
        }
    }
}

#[test]
fn malformed_responses_are_counted_and_only_acks_complete() {
    let mut node = node();
    let write = sent(&mut node, PEER, write_bdt(), at(0));
    // A malformed result never completes an exchange.
    let short = raw(0x00, &[0x00]);
    assert_eq!(
        answer(&mut node, &short, PEER, at(1)),
        Inbound::Unmatched {
            peer: PEER,
            code: None,
            late: false
        }
    );
    assert!(settled(
        &answer(&mut node, &result(SUCCESS), PEER, at(2)),
        write
    ));
    // A malformed ACK completes its read with the codec's error.
    let read = sent(&mut node, PEER, read_bdt(), at(3));
    let ragged = raw(0x03, &[0x00; 7]);
    assert!(matches!(
        answer(&mut node, &ragged, PEER, at(4)),
        Inbound::Matched { exchange, message: Err(_), accepted: false, settled: true }
            if exchange == read
    ));
    // An ACK the policy refuses completes it as received, marked refused
    // so the caller fails the exchange without judging the table again,
    // and uncounted.
    let read = sent(&mut node, PEER, read_bdt(), at(5));
    let rows = [
        BdtEntry::new(PEER, Ipv4Addr::BROADCAST),
        BdtEntry::new(REMOTE, Ipv4Addr::BROADCAST),
    ];
    let table = BdtEntries::from_entries(&rows);
    let big = wire(Bvll::ReadBroadcastDistributionTableAck(table));
    assert_eq!(
        node.receive(&datagram(&big, PEER, UNICAST), at(6), &mut OneRow),
        Inbound::Matched {
            exchange: read,
            message: Ok(Bvll::ReadBroadcastDistributionTableAck(table)),
            accepted: false,
            settled: true
        }
    );
    // One row passes the policy, accepted.
    let read = sent(&mut node, PEER, read_bdt(), at(6));
    let small = wire(Bvll::ReadBroadcastDistributionTableAck(
        BdtEntries::from_entries(&rows[..1]),
    ));
    let inbound = node.receive(&datagram(&small, PEER, UNICAST), at(6), &mut OneRow);
    assert!(settled(&inbound, read));
    // Nothing waiting: a malformed ACK is only malformed.
    assert!(matches!(
        answer(&mut node, &ragged, PEER, at(7)),
        Inbound::Unmatched {
            code: None,
            late: false,
            ..
        }
    ));
    let counters = node.take_counters();
    assert_eq!(counters.malformed_responses, 4);
    assert_eq!(counters.write_bdt.malformed_responses, 1);
    assert_eq!(counters.read_bdt.malformed_responses, 2);
    assert_eq!(
        (
            counters.read_bdt.acknowledgements,
            counters.write_bdt.results
        ),
        (1, 1)
    );
    assert_eq!(counters.unmatched_responses, 0);
}

#[test]
fn the_action_queue_never_overflows() {
    let mut node: Node<8> = Node::new(link(Mode::Normal), Instant::ZERO).unwrap();
    let mut exchanges = Vec::new();
    for host in 0..8 {
        let peer = addr(192, 168, 1, 100 + host, PORT);
        exchanges.push(sent(&mut node, peer, read_bdt(), at(0)));
    }
    // All eight end at once, and the late wake catches every one.
    node.on_deadline(at(60_000));
    let ended: Vec<_> = exchanges
        .iter()
        .map(|exchange| Action::ExchangeEnded {
            exchange: *exchange,
        })
        .collect();
    assert_eq!(drain(&mut node), ended);
}

#[test]
fn time_never_runs_backwards() {
    let mut node = node();
    let exchange = sent(&mut node, PEER, read_bdt(), at(10_000));
    // An earlier time is taken as the latest seen.
    assert!(!node.exchange_sent(exchange, at(0)));
    node.on_deadline(at(1));
    assert!(drain(&mut node).is_empty());
    node.on_deadline(at(13_000));
    assert_eq!(drain(&mut node), [Action::ExchangeEnded { exchange }]);
}
