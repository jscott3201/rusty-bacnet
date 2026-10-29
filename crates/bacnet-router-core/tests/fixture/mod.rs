//! A router on virtual time for the engine suites, with a [`Host`] that
//! records what it is asked and told, and owned copies of every send.
//!
//! The standard fixture has two ports: port 0 on network 1, a B/IP-like
//! link with 6-octet MACs, and port 1 on network 2, an MS/TP-like link
//! with 1-octet MACs. The router is [`OWN_0`] on port 0 and [`OWN_1`] on
//! port 1. Node A sits on port 0 at [`NODE_A`]; router R7 sits on port 1 at
//! [`R7`].

#![allow(dead_code)]

use bacnet_router_core::npdu::Mac;
use bacnet_router_core::npdu::NpduLimits;
use bacnet_router_core::npdu::{Address, NetworkPriority, Npdu};
use bacnet_router_core::npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_router_core::router::{
    Action, Cause, Claim, ControlClass, Counters, Dest, Discard, Event, Frame, GroupClassifier,
    Groups, Host, Kind, Options, PortConfig, Received, SendRefusal,
};
use bacnet_router_core::table::ClaimCounters;
use bacnet_router_core::{Duration, Instant, PortId, Router, Storage};

/// Up to 4 ports, with the hosted router's table and discovery sizes: 256
/// routes, 1,024 hold-down records, 256 searches.
pub type Store<G = Groups> = Storage<4, 256, 1024, 256, 1497, G>;
/// The router over it.
pub type TestRouter<G = Groups> = Router<Box<Store<G>>, 4, 256, 1024, 256, 1497, G>;

/// The router's MAC on port 0.
pub const OWN_0: &[u8] = &[10, 0, 0, 1, 0xBA, 0xC0];
/// The router's MAC on port 1.
pub const OWN_1: &[u8] = &[0x01];
/// Node A, on port 0.
pub const NODE_A: &[u8] = &[10, 0, 0, 9, 0xBA, 0xC0];
/// Router R7, on port 1.
pub const R7: &[u8] = &[0x07];

/// `n` seconds after the epoch.
pub const fn at(seconds: u64) -> Instant {
    Instant::from_secs(seconds)
}

/// `n` milliseconds after the epoch.
pub const fn at_ms(millis: u64) -> Instant {
    Instant::from_millis(millis)
}

/// `n` seconds.
pub const fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// The hosted options with what `set` changes: `Options` is
/// `#[non_exhaustive]`, so a test can't spell it out.
pub fn options(set: impl FnOnce(&mut Options)) -> Options {
    let mut options = Options::HOSTED;
    set(&mut options);
    options
}

/// The router's counts: zero but for what `set` changes.
pub fn router_counts(set: impl FnOnce(&mut Counters)) -> Counters {
    let mut counts = Counters::default();
    set(&mut counts);
    counts
}

/// The table's claim counts: zero but for what `set` changes.
pub fn claim_counts(set: impl FnOnce(&mut ClaimCounters)) -> ClaimCounters {
    let mut counts = ClaimCounters::default();
    set(&mut counts);
    counts
}

/// A port on `network` at `mac`, carrying 1,497 octets each way.
pub fn port_on(network: u16, mac: &[u8]) -> PortConfig {
    PortConfig::new(
        network,
        Mac::from_slice(mac).unwrap(),
        NpduLimits::both(1497),
        Groups::None,
    )
}

/// The two standard ports.
pub fn two_ports() -> [PortConfig; 2] {
    [port_on(1, OWN_0), port_on(2, OWN_1)]
}

/// A router on `ports` with `options`, built at t = 0.
pub fn router_with<G: GroupClassifier>(ports: &[PortConfig<G>], options: Options) -> TestRouter<G> {
    Router::new(Box::new(Store::new()), ports, options, Instant::ZERO).unwrap()
}

/// The standard router, with today's options.
pub fn router() -> TestRouter {
    router_with(&two_ports(), Options::HOSTED)
}

/// What a [`Claim`] said, owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub class: ControlClass,
    pub port: PortId,
    pub port_network: u16,
    pub source: Vec<u8>,
    pub message_type: NetworkMessageType,
    pub vendor_id: Option<u16>,
    pub snet: Option<u16>,
    pub dnet: Option<u16>,
    pub targets: Vec<u16>,
    pub targets_omitted: bool,
}

/// A host that records what it is asked and told.
#[derive(Debug, Default)]
pub struct Recorder {
    /// Every claim asked about, in order.
    pub asked: Vec<Asked>,
    /// Every event, in order.
    pub events: Vec<Event>,
    /// Classes to refuse.
    pub deny: Vec<ControlClass>,
}

impl Host for Recorder {
    fn authorize(&mut self, claim: &Claim<'_>) -> bool {
        self.asked.push(Asked {
            class: claim.class,
            port: claim.port,
            port_network: claim.port_network,
            source: claim.source.to_vec(),
            message_type: claim.message_type,
            vendor_id: claim.vendor_id,
            snet: claim.snet,
            dnet: claim.dnet,
            targets: claim.targets.iter().collect(),
            targets_omitted: claim.targets_omitted,
        });
        !self.deny.contains(&claim.class)
    }

    fn observe(&mut self, event: Event) {
        self.events.push(event);
    }
}

/// A send, owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub port: u8,
    /// `None` for a broadcast.
    pub to: Option<Vec<u8>>,
    pub npdu: Vec<u8>,
    pub priority: NetworkPriority,
    pub kind: Kind,
    pub cause: Cause,
}

impl Sent {
    /// The NPDU, parsed.
    pub fn parsed(&self) -> Npdu<'_> {
        Npdu::decode(&self.npdu).unwrap()
    }

    /// Whether it went out as a broadcast.
    pub fn is_broadcast(&self) -> bool {
        self.to.is_none()
    }
}

/// What a receive returned, owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Local {
        apdu: Vec<u8>,
        apdu_offset: usize,
        source: Option<(u16, Vec<u8>)>,
        group: bool,
        global: bool,
        expecting_reply: bool,
        priority: NetworkPriority,
    },
    ToRouter(Vec<u8>),
    Handled,
    Discarded(Discard),
}

impl Outcome {
    fn of(received: Received<'_>) -> Self {
        match received {
            Received::Local(local) => Self::Local {
                apdu: local.apdu.to_vec(),
                apdu_offset: local.apdu_offset,
                source: local
                    .source
                    .map(|source| (source.network, source.mac.to_vec())),
                group: local.group,
                global: local.global,
                expecting_reply: local.expecting_reply,
                priority: local.priority,
            },
            Received::ToRouter(npdu) => {
                let mut out = vec![0; npdu.encoded_len()];
                npdu.encode_into(&mut out).unwrap();
                Self::ToRouter(out)
            }
            Received::Handled => Self::Handled,
            Received::Discarded(reason) => Self::Discarded(reason),
        }
    }
}

/// Every action the router has, polled until idle.
pub fn drain<G: GroupClassifier>(router: &mut TestRouter<G>) -> Vec<Sent> {
    let mut sent = Vec::new();
    loop {
        match router.poll_action() {
            Action::Idle => return sent,
            Action::Send(send) => sent.push(Sent {
                port: send.port.0,
                to: match send.to {
                    Dest::Unicast(mac) => Some(mac.to_vec()),
                    Dest::Broadcast => None,
                },
                npdu: send.npdu.to_vec(),
                priority: send.priority,
                kind: send.kind,
                cause: send.cause,
            }),
        }
        assert!(sent.len() < 1_000, "the router never went idle");
    }
}

/// Polls until idle, refusing the sends `refuse` picks.
pub fn drain_refusing<G: GroupClassifier>(
    router: &mut TestRouter<G>,
    mut refuse: impl FnMut(usize, &Sent) -> Option<SendRefusal>,
) -> Vec<Sent> {
    let mut sent = Vec::new();
    loop {
        let one = match router.poll_action() {
            Action::Idle => return sent,
            Action::Send(send) => Sent {
                port: send.port.0,
                to: match send.to {
                    Dest::Unicast(mac) => Some(mac.to_vec()),
                    Dest::Broadcast => None,
                },
                npdu: send.npdu.to_vec(),
                priority: send.priority,
                kind: send.kind,
                cause: send.cause,
            },
        };
        if let Some(why) = refuse(sent.len(), &one) {
            router.refused(why);
        }
        sent.push(one);
        assert!(sent.len() < 1_000, "the router never went idle");
    }
}

/// Hand `npdu` to the router as port `port` received it from `source`, as
/// a unicast, and poll until idle.
pub fn deliver<G: GroupClassifier>(
    router: &mut TestRouter<G>,
    host: &mut impl Host,
    port: u8,
    source: &[u8],
    npdu: &[u8],
    now: Instant,
) -> (Outcome, Vec<Sent>) {
    deliver_frame(router, host, port, source, false, npdu, now)
}

/// The same, as a link broadcast.
pub fn deliver_broadcast<G: GroupClassifier>(
    router: &mut TestRouter<G>,
    host: &mut impl Host,
    port: u8,
    source: &[u8],
    npdu: &[u8],
    now: Instant,
) -> (Outcome, Vec<Sent>) {
    deliver_frame(router, host, port, source, true, npdu, now)
}

/// The same, with the link group flag given.
pub fn deliver_frame<G: GroupClassifier>(
    router: &mut TestRouter<G>,
    host: &mut impl Host,
    port: u8,
    source: &[u8],
    link_group: bool,
    npdu: &[u8],
    now: Instant,
) -> (Outcome, Vec<Sent>) {
    let frame = Frame {
        port: PortId(port),
        source,
        link_group,
        npdu,
    };
    let outcome = Outcome::of(router.receive(frame, now, host));
    (outcome, drain(router))
}

/// `npdu`, written.
pub fn bytes(npdu: &Npdu<'_>) -> Vec<u8> {
    let mut out = vec![0; npdu.encoded_len()];
    let len = npdu.encode_into(&mut out).unwrap();
    assert_eq!(len, out.len());
    out
}

/// An address.
pub fn addr(network: u16, mac: &[u8]) -> Address<'_> {
    Address { network, mac }
}

/// A confirmed request APDU (PDU type 0).
pub const CONFIRMED: &[u8] = &[0x00, 0x05, 0x01, 0x0C];
/// An unconfirmed request APDU (PDU type 1): Who-Is.
pub const UNCONFIRMED: &[u8] = &[0x10, 0x08];

/// An APDU for `dnet`/`dadr` with hop count 255.
pub fn apdu_to(dnet: u16, dadr: &[u8], apdu: &[u8]) -> Vec<u8> {
    let mut npdu = Npdu::apdu(apdu);
    npdu.destination = Some(addr(dnet, dadr));
    bytes(&npdu)
}

/// A network message of `kind` with `data` and no addresses.
pub fn message(kind: NetworkMessageType, data: &[u8]) -> Vec<u8> {
    bytes(&Npdu::message(kind, data))
}

/// Network numbers as a list.
pub fn networks(list: &[u16]) -> Vec<u8> {
    list.iter()
        .flat_map(|network| network.to_be_bytes())
        .collect()
}

/// An I-Am-Router for `list`.
pub fn i_am(list: &[u16]) -> Vec<u8> {
    message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &networks(list))
}

/// A reject's data.
pub fn reject_data(reason: RejectMessageReason, network: u16) -> Vec<u8> {
    let [high, low] = network.to_be_bytes();
    vec![reason.to_raw(), high, low]
}

/// The reject the router sends: reason and network, with the destination
/// it carries.
pub fn sent_reject(sent: &Sent) -> (RejectMessageReason, u16, Option<(u16, Vec<u8>)>) {
    let npdu = sent.parsed();
    assert_eq!(
        npdu.message_type,
        Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
    );
    assert_eq!(npdu.hop_count, 255, "a reject starts with a full hop count");
    assert!(npdu.source.is_none());
    let [reason, high, low] = *npdu.payload else {
        panic!("a reject's data is three octets");
    };
    (
        RejectMessageReason::from_raw(reason),
        u16::from_be_bytes([high, low]),
        npdu.destination.map(|d| (d.network, d.mac.to_vec())),
    )
}

/// The networks an I-Am-Router (or a list message) names.
pub fn listed(sent: &Sent) -> Vec<u16> {
    sent.parsed()
        .payload
        .chunks(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect()
}

/// R7 serving network 9 through port 1, learned at `now`.
pub fn learn_9_via_r7<G: GroupClassifier>(router: &mut TestRouter<G>, now: Instant) {
    assert!(
        router
            .table_mut()
            .add_learned(9, PortId(1), Mac::from_slice(R7).unwrap(), now)
    );
}
