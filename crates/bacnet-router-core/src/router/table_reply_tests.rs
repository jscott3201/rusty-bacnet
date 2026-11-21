//! Exercise two queued table replies independently of the public driver's
//! requirement to drain pending output between calls to receive.
use super::*;
use crate::router::{Action, Groups, Options, Router, SendRefusal};
use crate::{Instant, npdu::NpduLimits};

#[test]
fn a_refused_routed_table_chunk_can_only_end_its_requesters_continuation() {
    for refused_index in [0, 2] {
        let mut storage: Storage<1, 256, 0, 1> = Storage::new();
        let port = PortConfig::new(
            1,
            Mac::from_slice(&[1]).unwrap(),
            NpduLimits::both(501),
            Groups::None,
        );
        let mut engine =
            Router::new(&mut storage, &[port], Options::HOSTED, Instant::ZERO).unwrap();
        let hop = Mac::from_slice(&[2]).unwrap();
        for network in 100..355 {
            engine
                .table_mut()
                .add_learned(network, PortId(0), hop, Instant::ZERO);
        }
        for requester in [1, 2] {
            let destination = Address {
                network: 80 + u16::from(requester),
                mac: &[requester],
            };
            engine.store().out.push(Task::table_ack(
                PortId(0),
                hop.as_slice(),
                Some(destination),
                None,
            ));
        }
        let mut second_rows = 0;
        let mut index = 0;
        while let Action::Send(send) = engine.poll_action() {
            let ack = Npdu::decode(send.npdu).unwrap();
            let requester = ack.destination.unwrap();
            assert!(matches!(
                (requester.network, requester.mac),
                (81, [1]) | (82, [2])
            ));
            if requester.network == 82 {
                second_rows += usize::from(RoutingEntries::parse(ack.payload).unwrap().count());
            }
            if index == refused_index {
                engine.refused(SendRefusal::Full);
            }
            index += 1;
        }
        assert_eq!(second_rows, 256);
        assert_eq!(engine.counters().refused, 1);
    }
}
