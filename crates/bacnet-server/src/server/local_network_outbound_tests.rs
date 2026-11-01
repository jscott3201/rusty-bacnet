//! Outbound traffic the server starts toward this device's own network
//! number (#1358). A write in another device whose binding is routed through
//! that number goes there as a local request: a unicast to the device's MAC
//! with no DNET. The answer from that MAC with no SNET completes it, and so
//! does one a router relays back with this network as its SNET (#1465).
//! Both the Command action and the Channel member take that route. A binding
//! routed to another network, or any binding while the number is unknown,
//! still goes through its router with the DNET it names. An answer to a
//! request goes back by the route the request came in on, whatever its SNET
//! names.
//!
//! Device 9 is bound at the harness peer on the network a case names, behind
//! `ROUTER`. CMD-1's list 1 writes AO-1 there, then the local AO-2; CH-5
//! writes only AO-1 there. The number is published on the server's layer as
//! its Number worker would. The clock is paused.
use super::channel_wire_tests::{channel, member, settled, write_channel};
use super::command_action_wire_tests::{ao, idle, outputs, write_pv};
use super::command_remote_write_tests::{ack, deliver, device, remote_write, start_unbound};
use super::command_run_stop_tests::db_flags;
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::channel::MemberDatatype;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::WriteStatus;
use bacnet_types::network_number::NetworkNumber;

/// The number of the network this device is attached to.
const THIS_NETWORK: u16 = 7;
const REMOTE_NETWORK: u16 = 5;
const ROUTER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];

/// Which of the server's objects makes the write.
#[derive(Clone, Copy, Debug)]
enum Writer {
    Command,
    Channel,
}

/// Where the WriteProperty went: the link MAC and the NPDU's DNET, if any.
#[derive(Debug, PartialEq, Eq)]
struct Sent {
    link: MacAddr,
    dnet: Option<u16>,
}

/// A WriteProperty sent on this network to the harness peer.
fn local() -> Sent {
    Sent {
        link: MacAddr::from_slice(&PEER),
        dnet: None,
    }
}

/// A WriteProperty sent to `ROUTER` for the harness peer on `network`.
fn routed(network: u16) -> Sent {
    Sent {
        link: MacAddr::from_slice(&ROUTER),
        dnet: Some(network),
    }
}

async fn start(writer: Writer) -> Harness {
    match writer {
        Writer::Command => start_unbound().await,
        Writer::Channel => {
            Harness::start_with(ServerConfig::default(), |db| {
                outputs(db);
                let remote = BACnetDeviceObjectPropertyReference {
                    device_identifier: Some(device(9)),
                    ..member(ao(1), PropertyIdentifier::PRESENT_VALUE)
                };
                let mut channel = channel(5, 21, vec![(remote.clone(), 0)]);
                // As an earlier distribution's read would have, so the write
                // is the only request sent (#1342).
                let runs = channel.command_runner_mut().unwrap();
                runs.remember_member_datatype(0, &remote, Some(MemberDatatype::Real));
                db.add(Box::new(channel)).unwrap();
            })
            .await
        }
    }
}

/// How the device's answer comes back.
#[derive(Clone, Copy, Debug)]
enum Answer {
    /// The way the request went: from the device's own MAC for a local
    /// request, through `ROUTER` with the device's SNET otherwise.
    AsSent,
    /// Through `ROUTER`, with this network as its SNET and the device's MAC
    /// as its SADR, as a router here relays it.
    Relayed,
}

/// Where Device 9 is bound.
#[derive(Clone, Copy, Debug)]
enum Bound {
    /// At the peer on this network number, behind `ROUTER`.
    Routed(u16),
    /// At the peer on this link, with no router.
    Local,
}

/// A write to Device 9 that has gone out and waits for its answer.
struct Write {
    h: Harness,
    writer: Writer,
    invoke_id: u8,
    sent: Sent,
}

/// Publish `number` as this network's own, as the server's Number worker
/// would.
fn publish(h: &Harness, number: u16) {
    h.server
        .test_network()
        .local_network_number()
        .publish(NetworkNumber::configured(number).unwrap());
}

/// Start `writer`'s write to Device 9, bound as `bound`, with `this_network`
/// published as the local network's number, and return it once it is out.
async fn send_write(writer: Writer, this_network: Option<u16>, bound: Bound) -> Write {
    let mut h = start(writer).await;
    let binding = match bound {
        Bound::Routed(network) => DeviceBinding::routed(device(9), network, PEER, ROUTER),
        Bound::Local => DeviceBinding::local(device(9), PEER),
    };
    h.server
        .device_bindings
        .write()
        .await
        .insert_configured(binding.unwrap(), |_| false)
        .unwrap();
    if let Some(number) = this_network {
        publish(&h, number);
    }
    match writer {
        Writer::Command => write_pv(&mut h, 1, 1).await.unwrap(),
        Writer::Channel => write_channel(&mut h, 5, &PropertyValue::Real(80.0), Some(8))
            .await
            .unwrap(),
    }
    let invoke_id = remote_write(&h).await;
    let frame = h
        .server
        .test_network()
        .transport()
        .sent()
        .unicasts()
        .into_iter()
        .find(|frame| matches!(frame.apdu(), Apdu::ConfirmedRequest(_)))
        .expect("the WriteProperty frame");
    let destination = frame.decode_npdu().destination;
    if let Some(to) = &destination {
        assert_eq!(to.mac_address.as_slice(), PEER, "{writer:?}: the DADR");
    }
    let sent = Sent {
        link: frame.mac.clone(),
        dnet: destination.map(|to| to.network),
    };
    Write {
        h,
        writer,
        invoke_id,
        sent,
    }
}

impl Write {
    /// Deliver `answer` to the write from link MAC `link`, with `snet` and
    /// the device's MAC as its SNET/SADR when given.
    async fn deliver(&self, answer: &Apdu, link: &[u8], snet: Option<u16>) {
        let source = snet.map(|network| NpduAddress {
            network,
            mac_address: MacAddr::from_slice(&PEER),
        });
        deliver(&self.h, answer, link, source).await;
    }

    /// A Reject for the write: if one completed it, the write would fail.
    fn reject(&self) -> Apdu {
        Apdu::Reject(bacnet_encoding::apdu::RejectPdu {
            invoke_id: self.invoke_id,
            reject_reason: bacnet_types::enums::RejectReason::OTHER,
        })
    }

    /// Wait for the write to end, check it succeeded, and return the router
    /// the server learned for `network`, if any.
    async fn succeeded(mut self, network: u16) -> Option<MacAddr> {
        match self.writer {
            Writer::Command => {
                idle(&self.h, 1).await;
                assert_eq!(
                    db_flags(&self.h, 1).await,
                    [true, true],
                    "{:?}",
                    self.writer
                );
            }
            Writer::Channel => {
                assert_eq!(settled(&mut self.h, 5).await, WriteStatus::SUCCESSFUL);
            }
        }
        assert_eq!(self.h.server.notification_transactions.active_count(), 0);
        let learned = self.h.server.learned_routers.lock().await;
        learned.cached_router(network)
    }
}

/// Run `writer`'s write to Device 9, bound at the peer on `bound_on` behind
/// `ROUTER`, with `this_network` published as the local network's number.
/// The answer comes back as `answer` says, and the write succeeds.
async fn write_through(
    writer: Writer,
    this_network: Option<u16>,
    bound_on: u16,
    answer: Answer,
) -> Sent {
    let write = send_write(writer, this_network, Bound::Routed(bound_on)).await;
    let (link, snet) = match answer {
        Answer::AsSent => (write.sent.link.clone(), write.sent.dnet),
        Answer::Relayed => (MacAddr::from_slice(&ROUTER), this_network),
    };
    write.deliver(&ack(write.invoke_id), &link, snet).await;
    let sent = Sent {
        link: write.sent.link.clone(),
        dnet: write.sent.dnet,
    };
    let learned = write.succeeded(this_network.unwrap_or(bound_on)).await;
    if this_network.is_some() {
        // An answer relayed from this network teaches no route to it.
        assert_eq!(learned, None, "{writer:?}");
    }
    sent
}

#[tokio::test(start_paused = true)]
async fn command_write_bound_through_this_network_goes_without_a_dnet() {
    let writer = Writer::Command;
    assert_eq!(
        write_through(writer, Some(THIS_NETWORK), THIS_NETWORK, Answer::AsSent).await,
        local()
    );
    assert_eq!(
        write_through(writer, Some(THIS_NETWORK), REMOTE_NETWORK, Answer::AsSent).await,
        routed(REMOTE_NETWORK)
    );
    // While the number is unknown the binding is taken as written.
    assert_eq!(
        write_through(writer, None, THIS_NETWORK, Answer::AsSent).await,
        routed(THIS_NETWORK)
    );
}

#[tokio::test(start_paused = true)]
async fn channel_member_bound_through_this_network_is_written_without_a_dnet() {
    let writer = Writer::Channel;
    assert_eq!(
        write_through(writer, Some(THIS_NETWORK), THIS_NETWORK, Answer::AsSent).await,
        local()
    );
    assert_eq!(
        write_through(writer, Some(THIS_NETWORK), REMOTE_NETWORK, Answer::AsSent).await,
        routed(REMOTE_NETWORK)
    );
    assert_eq!(
        write_through(writer, None, THIS_NETWORK, Answer::AsSent).await,
        routed(THIS_NETWORK)
    );
}

/// Once the number is known, a write sent to the device's MAC is completed
/// by its answer relayed back through a router with this network as its SNET
/// and that MAC as its SADR, as by a direct one (#1465).
#[tokio::test(start_paused = true)]
async fn a_local_write_is_completed_by_its_answer_relayed_with_this_networks_snet() {
    for writer in [Writer::Command, Writer::Channel] {
        assert_eq!(
            write_through(writer, Some(THIS_NETWORK), THIS_NETWORK, Answer::Relayed).await,
            local()
        );
    }
}

/// A relayed answer stands for the device only when its SNET is this
/// network's known number. A Reject relayed with another network's SNET, or
/// relayed with SNET 7 while the number is unknown, neither ends a write sent
/// to the device's MAC nor teaches a router; the device's own SimpleACK after
/// it completes the write.
#[tokio::test(start_paused = true)]
async fn a_relayed_answer_from_another_network_or_an_unknown_number_leaves_a_local_write_open() {
    for writer in [Writer::Command, Writer::Channel] {
        for (this_network, bound, snet) in [
            (
                Some(THIS_NETWORK),
                Bound::Routed(THIS_NETWORK),
                REMOTE_NETWORK,
            ),
            (None, Bound::Local, THIS_NETWORK),
        ] {
            let write = send_write(writer, this_network, bound).await;
            assert_eq!(write.sent, local(), "{writer:?}, {bound:?}");
            write.deliver(&write.reject(), &ROUTER, Some(snet)).await;
            write.deliver(&ack(write.invoke_id), &PEER, None).await;
            assert_eq!(write.succeeded(snet).await, None, "{writer:?}, {bound:?}");
        }
    }
}

/// A write routed to this network while its number was unknown keeps its
/// routed key. Learned before the answer comes back, the number does not
/// stop that answer, relayed with this network as its SNET, from completing
/// the write, as it would have before (#1465); and it teaches no router for
/// this network.
#[tokio::test(start_paused = true)]
async fn a_write_routed_before_the_number_was_learned_completes_on_its_relayed_answer() {
    for writer in [Writer::Command, Writer::Channel] {
        let write = send_write(writer, None, Bound::Routed(THIS_NETWORK)).await;
        assert_eq!(write.sent, routed(THIS_NETWORK), "{writer:?}");
        publish(&write.h, THIS_NETWORK);
        write
            .deliver(&ack(write.invoke_id), &ROUTER, Some(THIS_NETWORK))
            .await;
        assert_eq!(write.succeeded(THIS_NETWORK).await, None, "{writer:?}");
    }
}

/// A ReadProperty that a router delivers with this network's own number as
/// its SNET is answered back through that router, with that DNET: the answer
/// retraces the request's route, as the requester expects it.
#[tokio::test(start_paused = true)]
async fn an_answer_to_a_request_routed_from_this_network_goes_back_by_its_route() {
    let h = start_unbound().await;
    h.server
        .test_network()
        .local_network_number()
        .publish(NetworkNumber::configured(THIS_NETWORK).unwrap());
    let mut service = BytesMut::new();
    bacnet_services::read_property::ReadPropertyRequest {
        object_identifier: device(856),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut service);
    let request = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 42,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: service.freeze(),
    });
    let requester = NpduAddress {
        network: THIS_NETWORK,
        mac_address: MacAddr::from_slice(&PEER),
    };
    deliver(&h, &request, &ROUTER, Some(requester.clone())).await;
    let log = h.server.test_network().transport().sent();
    let answer = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let unicasts = log.unicasts();
            if let Some(frame) = unicasts
                .into_iter()
                .find(|frame| matches!(frame.apdu(), Apdu::ComplexAck(_)))
            {
                return frame;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the ComplexACK");
    assert_eq!(answer.mac.as_slice(), ROUTER);
    assert_eq!(answer.decode_npdu().destination, Some(requester));
}
