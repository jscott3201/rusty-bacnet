//! FD_BBMD_Address naming the BBMD by host name (#1852): looked up at
//! activation and before each registration attempt, with failures counted
//! and the last good address kept. A foreign device on a recording socket,
//! under paused time, with a table for a resolver.

use std::io;
use std::net::{IpAddr, Ipv6Addr};

use bacnet_bip_core::node::Mode;
use bacnet_types::bip_port::ForeignDeviceChanges;
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::super::send_sink::{SendSink, Sent};
use super::super::*;
use super::ForeignTarget;
use crate::bbmd_lookup::table::{Answer, Names};
use crate::bbmd_lookup::{BbmdLookupFailure, BbmdResolver};

const OWN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), PORT);
const BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), PORT);
const PORT: u16 = 0xBAC0;
const A: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 1);
const B: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 2);
const C: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 1);
const NAME: &str = "bbmd.test";

/// A foreign device at [`OWN`] registering with [`A`], never answered, so
/// it retries: each attempt times out after 3 s, and the next comes by the
/// renewal interval, half the time-to-live.
struct Device {
    target: Arc<ForeignTarget>,
    node: Arc<NodeHandle>,
    sink: SendSink,
    names: Arc<Names>,
    driver: JoinHandle<()>,
}

impl Device {
    async fn new(ttl: u16) -> Self {
        Self::configured((A, PORT).into(), ttl).await
    }

    /// A device configured with `bbmd`, as a start builds one. A host name
    /// is looked up on the first await after this returns.
    async fn configured(bbmd: ForeignBbmd, ttl: u16) -> Self {
        let sink = SendSink::default();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let socket = Arc::new(BipSocket::recording(socket, sink.clone()));
        let config = ForeignDeviceConfig {
            bbmd,
            ttl,
            renewal_interval: None,
        };
        let names = Arc::new(Names::default());
        let mut transport = BipTransport::new(*OWN.ip(), PORT, *BROADCAST.ip());
        transport.register_as_foreign_device(config.clone());
        transport.set_release_previous_bbmd(true);
        transport.set_bbmd_resolver(Arc::clone(&names) as Arc<dyn BbmdResolver>);
        let target = Arc::clone(transport.foreign_device.as_ref().unwrap());
        let mode = Mode::Foreign(config.engine(config.interval().unwrap()));
        let link = test_support::test_link(OWN, BROADCAST, mode);
        let node = Arc::new(NodeHandle::new(link, socket, Arc::default(), Arc::default()).unwrap());
        target.attach(&node, &config).unwrap();
        let driver = tokio::spawn(Arc::clone(&node).drive());
        Self {
            target,
            node,
            sink,
            names,
            driver,
        }
    }

    /// Stop the node and start another on the same target, as a restart
    /// of the transport does.
    async fn restart(&mut self) {
        self.node.stop();
        self.target.detach();
        self.driver.abort();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let socket = Arc::new(BipSocket::recording(socket, self.sink.clone()));
        let config = self.target.config();
        let mode = Mode::Foreign(config.engine(config.interval().unwrap()));
        let link = test_support::test_link(OWN, BROADCAST, mode);
        self.node =
            Arc::new(NodeHandle::new(link, socket, Arc::default(), Arc::default()).unwrap());
        self.target.attach(&self.node, &config).unwrap();
        self.driver = tokio::spawn(Arc::clone(&self.node).drive());
    }

    fn view(&self) -> &dyn ForeignRegistration {
        &*self.target
    }

    fn activate(&self, host: BACnetHostAddress) -> Result<(), Error> {
        let mut changes = ForeignDeviceChanges::default();
        changes.bbmd = Some(BACnetHostNPort { host, port: PORT });
        self.view().activate(&changes)
    }

    fn name() -> BACnetHostAddress {
        BACnetHostAddress::Name(NAME.into())
    }

    /// Let the paused clock run `ms` milliseconds.
    async fn after(&self, ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    /// Let the clock run until the next registration attempt, and the
    /// lookup and any move it starts, then what was sent since the last
    /// look.
    async fn next_attempt(&self) -> Vec<Sent> {
        let mut sent = Vec::new();
        for _ in 0..200 {
            self.after(100).await;
            sent.extend(self.sink.take());
            if sent.iter().any(is_registration) {
                self.after(1).await;
                sent.extend(self.sink.take());
                return sent;
            }
        }
        panic!("no registration attempt within 20 s");
    }

    fn bbmd(&self) -> SocketAddrV4 {
        self.target.config().bbmd().expect("a BBMD address")
    }

    fn snapshot(&self) -> ForeignRegistrationSnapshot {
        self.node.snapshot().foreign_registration
    }
}

fn is_registration(sent: &Sent) -> bool {
    sent.bytes.get(1) == Some(&0x05)
}

/// Where each Register-Foreign-Device in `sent` went.
fn registrations(sent: &[Sent]) -> Vec<Ipv4Addr> {
    sent.iter()
        .filter(|sent| is_registration(sent))
        .map(|sent| *sent.to.ip())
        .collect()
}

fn at(ip: Ipv4Addr) -> SocketAddrV4 {
    SocketAddrV4::new(ip, PORT)
}

/// Activated, the name is looked up and the device moves to the address it
/// stands for: one Register-Foreign-Device goes there, none to the old BBMD
/// first, and the old BBMD is asked to drop the entry. The row reads back
/// the name.
#[tokio::test(start_paused = true)]
async fn a_name_is_looked_up_at_activation_and_the_device_moves_there() {
    let device = Device::new(60).await;
    device.after(1).await;
    assert_eq!(registrations(&device.sink.take()), [A]);
    // An IPv6 answer is no B/IP address: the first IPv4 one counts.
    let answers = [IpAddr::V6(Ipv6Addr::LOCALHOST), IpAddr::V4(B)];
    device.names.answer(NAME, Answer::addresses(answers));
    device.activate(Device::name()).unwrap();
    assert_eq!(
        device.view().bbmd(),
        BACnetHostNPort {
            host: Device::name(),
            port: PORT
        }
    );
    device.after(1).await;
    let sent = device.sink.take();
    // The attempt to the new address starts no second lookup.
    assert_eq!(registrations(&sent), [B]);
    assert_eq!(device.bbmd(), at(B));
    let mut release = vec![0x81, 0x08, 0x00, 0x0A];
    release.extend(OWN.ip().octets());
    release.extend(PORT.to_be_bytes());
    assert!(
        sent.iter()
            .any(|sent| sent.to == at(A) && sent.bytes == release),
        "{sent:?}"
    );
    let snapshot = device.snapshot();
    assert_eq!(
        (
            snapshot.bbmd_lookups,
            snapshot.bbmd_lookup_failures,
            snapshot.last_bbmd_lookup_failure
        ),
        (1, 0, None)
    );
    assert_eq!(device.names.calls(), 1);
}

/// Later lookups run alongside the registration attempts, whose attempt
/// still goes to the address in use. While that address is among the
/// answers the device stays; once it isn't, the device moves, and registers
/// at the new address at once.
#[tokio::test(start_paused = true)]
async fn attempts_look_the_name_up_again_and_move_the_device() {
    let device = Device::new(10).await;
    device.after(1).await;
    device.sink.take();
    device.names.answer(NAME, Answer::addresses([C, A]));
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    assert!(registrations(&device.sink.take()).is_empty());
    assert_eq!((device.names.calls(), device.bbmd()), (1, at(A)));

    // The attempt at 35 s looks it up again: A is still an answer.
    device.after(40_000).await;
    assert!(registrations(&device.sink.take()).iter().all(|ip| *ip == A));
    assert_eq!((device.names.calls(), device.bbmd()), (2, at(A)));

    // The one at 65 s finds only C: that attempt went to A, then the device
    // moves and registers with C at once.
    device.names.answer(NAME, Answer::addresses([C]));
    device.after(30_000).await;
    let sent = registrations(&device.sink.take());
    let moved = sent
        .iter()
        .position(|ip| *ip == C)
        .expect("a registration with C");
    assert!(
        moved > 0 && sent[..moved].iter().all(|ip| *ip == A),
        "{sent:?}"
    );
    assert!(sent[moved..].iter().all(|ip| *ip == C), "{sent:?}");
    assert_eq!((device.names.calls(), device.bbmd()), (3, at(C)));
    let snapshot = device.snapshot();
    assert_eq!(
        (snapshot.bbmd_lookups, snapshot.bbmd_lookup_failures),
        (3, 0)
    );
}

/// Outside an activation, the attempts look the name up at most once per
/// 30 s, however often they come: here every 5 s, unanswered.
#[tokio::test(start_paused = true)]
async fn attempts_look_the_name_up_at_most_once_per_30_s() {
    let device = Device::new(10).await;
    device.after(1).await;
    device.names.answer(NAME, Answer::addresses([A]));
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    assert_eq!(device.names.calls(), 1);
    device.after(29_000).await;
    assert!(registrations(&device.sink.take()).len() >= 5);
    assert_eq!(device.names.calls(), 1);
    // The attempt at 35 s is the first 30 s past that lookup.
    device.after(11_000).await;
    assert_eq!(device.names.calls(), 2);
    device.after(24_000).await;
    assert_eq!(device.names.calls(), 2);
    // An activation always looks it up.
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    assert_eq!(device.names.calls(), 3);
}

/// A lookup that fails, finds no IPv4 address, or only addresses the device
/// can't register with is counted with its reason, and the device keeps
/// registering where it did. A lookup that hangs is the only one running
/// until it times out. The next good answer moves the device and clears the
/// reason.
#[tokio::test(start_paused = true)]
async fn a_failed_lookup_is_counted_and_the_device_stays_where_it_was() {
    let device = Device::new(10).await;
    device.after(1).await;
    device.sink.take();
    let cases = [
        (
            Answer::Fails(io::ErrorKind::NotFound),
            BbmdLookupFailure::Error(io::ErrorKind::NotFound),
        ),
        (Answer::Addresses(vec![]), BbmdLookupFailure::NoAddress),
        (
            Answer::addresses([Ipv6Addr::LOCALHOST]),
            BbmdLookupFailure::NoAddress,
        ),
        // Its own address, then a broadcast and the unspecified address,
        // as a start would refuse them.
        (
            Answer::addresses([*OWN.ip()]),
            BbmdLookupFailure::Unusable((*OWN.ip()).into()),
        ),
        (
            Answer::addresses([*BROADCAST.ip(), Ipv4Addr::UNSPECIFIED]),
            BbmdLookupFailure::Unusable((*BROADCAST.ip()).into()),
        ),
    ];
    let count = cases.len() as u64;
    for (answer, failure) in cases {
        device.names.answer(NAME, answer);
        device.activate(Device::name()).unwrap();
        device.after(1).await;
        let snapshot = device.snapshot();
        assert_eq!(snapshot.last_bbmd_lookup_failure, Some(failure));
        assert_eq!(device.bbmd(), at(A), "{failure:?}");
    }
    let snapshot = device.snapshot();
    assert_eq!(
        (snapshot.bbmd_lookups, snapshot.bbmd_lookup_failures),
        (count, count)
    );
    assert!(registrations(&device.sink.take()).is_empty());

    // A lookup that hangs: the attempts meanwhile still go to A, and start
    // no lookup of their own, until it times out after 10 s.
    device.names.answer(NAME, Answer::Hangs);
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    assert_eq!(device.names.calls(), count as usize + 1);
    assert_eq!(registrations(&device.next_attempt().await), [A]);
    assert_eq!(device.names.calls(), count as usize + 1);
    device.after(9_000).await;
    let snapshot = device.snapshot();
    assert_eq!(
        snapshot.last_bbmd_lookup_failure,
        Some(BbmdLookupFailure::TimedOut)
    );
    assert_eq!(snapshot.bbmd_lookup_failures, count + 1);

    device.names.answer(NAME, Answer::addresses([B]));
    // The attempts within 30 s of the hung lookup's start keep A.
    let mut seen = Vec::new();
    for _ in 0..8 {
        seen.extend(registrations(&device.next_attempt().await));
        if seen.contains(&B) {
            break;
        }
    }
    assert!(seen.contains(&B), "{seen:?}");
    let snapshot = device.snapshot();
    assert_eq!(snapshot.last_bbmd_lookup_failure, None);
    assert_eq!(snapshot.bbmd_lookup_failures, count + 1);
    assert_eq!(device.bbmd(), at(B));
}

/// A lookup started for a name that an address has since replaced changes
/// nothing when it ends, and counts nothing; with an address, attempts look
/// nothing up. (A name replaced by another name: the next test.)
#[tokio::test(start_paused = true)]
async fn a_lookup_for_a_replaced_name_changes_nothing() {
    let device = Device::new(10).await;
    device.after(1).await;
    device.sink.take();
    let gate = Arc::new(Notify::new());
    device.names.answer(NAME, Answer::gated(&gate, [B]));
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    device.activate(BACnetHostAddress::Ip(C.into())).unwrap();
    gate.notify_one();
    device.after(1).await;
    assert_eq!(registrations(&device.sink.take()), [C]);
    assert_eq!(device.bbmd(), at(C));
    assert_eq!(
        device.view().bbmd(),
        BACnetHostNPort::from_socket_addr(at(C).into())
    );
    assert_eq!(device.snapshot().bbmd_lookups, 0);
    assert_eq!(registrations(&device.next_attempt().await), [C]);
    assert_eq!(device.names.calls(), 1);
}

/// One resolver call runs at a time, whatever it was started for: names
/// activated while one runs wait for it to end, and only the newest is
/// looked up then. The late answer for the first name moves nothing.
#[tokio::test(start_paused = true)]
async fn one_resolver_call_runs_at_a_time_and_the_newest_name_follows() {
    let device = Device::new(60).await;
    device.after(1).await;
    device.sink.take();
    let gate = Arc::new(Notify::new());
    device.names.answer(NAME, Answer::gated(&gate, [B]));
    device.names.answer("other.test", Answer::addresses([B]));
    device.names.answer("newest.test", Answer::addresses([C]));
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    for host in ["other.test", "newest.test"] {
        device
            .activate(BACnetHostAddress::Name(host.into()))
            .unwrap();
    }
    device.after(1).await;
    assert_eq!((device.names.calls(), device.bbmd()), (1, at(A)));
    gate.notify_one();
    device.after(1).await;
    assert_eq!(device.names.calls(), 2);
    assert_eq!(device.names.most_running(), 1);
    assert_eq!(device.bbmd(), at(C));
    assert_eq!(registrations(&device.sink.take()), [C]);
    assert_eq!(
        device.view().bbmd(),
        BACnetHostNPort {
            host: BACnetHostAddress::Name("newest.test".into()),
            port: PORT
        }
    );
    assert_eq!(device.snapshot().bbmd_lookups, 1);
}

/// A resolver that panics ends its lookup all the same: counted as
/// abandoned, and the slot freed, so a later attempt looks the name up and
/// moves the device.
#[tokio::test(start_paused = true)]
async fn a_panicking_resolver_frees_the_slot() {
    let device = Device::new(10).await;
    device.after(1).await;
    device.names.answer(NAME, Answer::Panics);
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    let snapshot = device.snapshot();
    assert_eq!(
        snapshot.last_bbmd_lookup_failure,
        Some(BbmdLookupFailure::Abandoned)
    );
    assert_eq!((device.names.calls(), device.bbmd()), (1, at(A)));
    device.names.answer(NAME, Answer::addresses([B]));
    device.after(40_000).await;
    assert_eq!((device.names.calls(), device.bbmd()), (2, at(B)));
    let snapshot = device.snapshot();
    assert_eq!(
        (
            snapshot.bbmd_lookup_failures,
            snapshot.last_bbmd_lookup_failure
        ),
        (1, None)
    );
}

/// A lookup dropped with its runtime, as at shutdown, ends all the same: it
/// is counted as abandoned and frees the slot, so an activation on another
/// runtime looks its name up at once.
#[test]
fn a_lookup_dropped_with_its_runtime_frees_the_slot() {
    let paused = || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap()
    };
    let runtime = paused();
    let device = runtime.block_on(async {
        let device = Device::new(60).await;
        device.names.answer(NAME, Answer::Hangs);
        device.activate(Device::name()).unwrap();
        device.after(1).await;
        device
    });
    assert_eq!(device.names.calls(), 1);
    drop(runtime);
    assert_eq!(
        device.snapshot().last_bbmd_lookup_failure,
        Some(BbmdLookupFailure::Abandoned)
    );
    paused().block_on(async {
        device.names.answer(NAME, Answer::addresses([B]));
        device.activate(Device::name()).unwrap();
        device.after(1).await;
    });
    assert_eq!((device.names.calls(), device.bbmd()), (2, at(B)));
}

/// A start looks the name up at its first registration attempt, however
/// recently the last lookup ran.
#[tokio::test(start_paused = true)]
async fn a_restart_looks_the_name_up_at_its_first_attempt() {
    let mut device = Device::new(60).await;
    device.after(1).await;
    device.names.answer(NAME, Answer::addresses([A]));
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    assert_eq!(device.names.calls(), 1);
    device.names.answer(NAME, Answer::addresses([B]));
    device.restart().await;
    device.after(1).await;
    assert_eq!((device.names.calls(), device.bbmd()), (2, at(B)));
}

/// A lookup that ends once the node has stopped changes and counts
/// nothing: a start looks the name up again.
#[tokio::test(start_paused = true)]
async fn a_lookup_ending_after_a_stop_counts_nothing() {
    let device = Device::new(60).await;
    device.after(1).await;
    let gate = Arc::new(Notify::new());
    device.names.answer(NAME, Answer::gated(&gate, [B]));
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    device.node.stop();
    gate.notify_one();
    device.after(1).await;
    assert_eq!(device.bbmd(), at(A));
    assert_eq!(device.snapshot().bbmd_lookups, 0);
}

/// A name activated while a call runs isn't looked up once that call ends
/// if the transport has stopped meanwhile: a stopped transport sends no
/// query, and a start looks the name up again.
#[tokio::test(start_paused = true)]
async fn no_lookup_follows_a_stop() {
    let device = Device::new(60).await;
    device.after(1).await;
    let gate = Arc::new(Notify::new());
    device.names.answer(NAME, Answer::gated(&gate, [B]));
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    device
        .activate(BACnetHostAddress::Name("other.test".into()))
        .unwrap();
    device.node.stop();
    gate.notify_one();
    device.after(1).await;
    assert_eq!((device.names.calls(), device.bbmd()), (1, at(A)));
}

/// A written name is checked for its form and port, and the check changes
/// nothing and looks nothing up.
#[tokio::test(start_paused = true)]
async fn a_written_name_is_checked_for_its_form() {
    let device = Device::new(60).await;
    let written = |host: &str, port| {
        let mut changes = ForeignDeviceChanges::default();
        changes.bbmd = Some(BACnetHostNPort {
            host: BACnetHostAddress::Name(host.into()),
            port,
        });
        changes
    };
    for (host, port) in [("bbmd test", PORT), ("-bbmd", PORT), ("", PORT), (NAME, 0)] {
        for error in [
            device.view().check(&written(host, port)).unwrap_err(),
            device.view().activate(&written(host, port)).unwrap_err(),
        ] {
            assert!(
                matches!(error, Error::Remote(RemoteError::Error { class, code })
                    if class == ErrorClass::PROPERTY && code == ErrorCode::VALUE_OUT_OF_RANGE),
                "{host:?}:{port}: {error:?}"
            );
        }
    }
    device.view().check(&written(NAME, PORT)).unwrap();
    device.after(1).await;
    assert_eq!(device.names.calls(), 0);
    assert_eq!(
        device.view().bbmd(),
        BACnetHostNPort::from_socket_addr(at(A).into())
    );
}

/// A move away and straight back, before the driver has sent the release
/// of the first BBMD (#2307): that release would delete the entry the
/// device is about to make there, and while it lived the registration there
/// would wait a retry. The move back drops it unsent, counted as
/// unanswered, so the device registers with its BBMD again at once and
/// releases only the BBMD it visited. Not about host names, but this file
/// has the paused-time device with a recording socket.
#[tokio::test(start_paused = true)]
async fn a_quick_move_back_drops_the_release_it_would_meet() {
    let device = Device::new(60).await;
    device.after(1).await;
    assert_eq!(registrations(&device.sink.take()), [A]);
    device.activate(BACnetHostAddress::Ip(B.into())).unwrap();
    device.activate(BACnetHostAddress::Ip(A.into())).unwrap();
    device.after(1).await;
    let sent = device.sink.take();
    assert_eq!(registrations(&sent), [A], "{sent:?}");
    let releases: Vec<Ipv4Addr> = sent
        .iter()
        .filter(|sent| sent.bytes.get(1) == Some(&0x08))
        .map(|sent| *sent.to.ip())
        .collect();
    assert_eq!(releases, [B]);
    let snapshot = device.snapshot();
    assert_eq!(
        (
            snapshot.previous_releases,
            snapshot.previous_release_unanswered
        ),
        (2, 1)
    );
}

/// A BBMD configured by address is never waited for (#2308): not at start,
/// and not while the first attempt is out, unanswered.
#[tokio::test(start_paused = true)]
async fn a_device_configured_by_address_never_waits() {
    let device = Device::new(60).await;
    assert!(!device.snapshot().waiting_for_bbmd);
    device.after(1).await;
    assert_eq!(registrations(&device.sink.take()), [A]);
    let snapshot = device.snapshot();
    assert_eq!(
        (snapshot.last_outcome, snapshot.registered),
        (Some(ForeignRegistrationOutcome::Pending), false)
    );
    assert!(!snapshot.waiting_for_bbmd);
}

/// A configured BBMD named by host name (#2308): the start runs with no
/// BBMD, looks the name up at once, and registers with what it stands for.
/// The row reads back the name, and the registration in use the address.
#[tokio::test(start_paused = true)]
async fn a_configured_name_is_looked_up_at_start_and_registers_there() {
    let named = ForeignBbmd::Name {
        host: NAME.into(),
        port: PORT,
    };
    let device = Device::configured(named, 60).await;
    device.names.answer(NAME, Answer::addresses([B]));
    device.after(1).await;
    assert_eq!(registrations(&device.sink.take()), [B]);
    assert_eq!(device.bbmd(), at(B));
    assert_eq!(
        device.view().bbmd(),
        BACnetHostNPort {
            host: Device::name(),
            port: PORT
        }
    );
    let snapshot = device.snapshot();
    assert_eq!(
        (snapshot.bbmd_lookups, snapshot.bbmd_lookup_failures),
        (1, 0)
    );
    // Nothing to release: the device had no BBMD before.
    assert_eq!(snapshot.previous_releases, 0);
}

/// A configured name that doesn't resolve still runs (#2308): no
/// registration is sent, broadcasts are held and then refused, the failure
/// is counted, and the name is looked up again every 30 s, since no
/// registration attempt comes to look it up alongside. The first answer
/// registers the device, and the broadcasts held meanwhile go to that BBMD
/// once the attempt resolves.
#[tokio::test(start_paused = true)]
async fn a_configured_name_that_fails_waits_and_is_tried_again_every_30_s() {
    let named = ForeignBbmd::Name {
        host: NAME.into(),
        port: PORT,
    };
    let device = Device::configured(named, 60).await;
    device.after(1).await;
    let snapshot = device.snapshot();
    assert_eq!(
        (
            snapshot.bbmd_lookups,
            snapshot.bbmd_lookup_failures,
            snapshot.last_bbmd_lookup_failure
        ),
        (
            1,
            1,
            Some(BbmdLookupFailure::Error(io::ErrorKind::NotFound))
        )
    );
    assert_eq!((snapshot.attempts, snapshot.registered), (0, false));
    assert_eq!(snapshot.next_attempt_in, None);
    assert!(snapshot.waiting_for_bbmd);
    for _ in 0..16 {
        device
            .node
            .send_broadcast(&[0x01, 0x00], None)
            .await
            .unwrap();
    }
    assert!(
        device
            .node
            .send_broadcast(&[0x01, 0x00], None)
            .await
            .is_err()
    );
    device.after(29_000).await;
    assert_eq!(device.names.calls(), 1);
    device.after(1_100).await;
    assert_eq!(device.names.calls(), 2);
    assert!(device.sink.take().is_empty());

    device.names.answer(NAME, Answer::addresses([A]));
    device.after(30_100).await;
    assert_eq!(device.names.calls(), 3);
    assert_eq!(registrations(&device.sink.take()), [A]);
    // A never answers: its attempt times out after 3 s, which releases the
    // broadcasts held, in order, to A.
    device.after(3_000).await;
    let held: Vec<_> = device
        .sink
        .take()
        .into_iter()
        .filter(|sent| sent.bytes.get(1) == Some(&0x09))
        .map(|sent| sent.to)
        .collect();
    assert_eq!(held, [at(A); 16]);
    let snapshot = device.snapshot();
    assert_eq!(
        (snapshot.bbmd_lookups, snapshot.bbmd_lookup_failures),
        (3, 2)
    );
    assert_eq!(snapshot.broadcasts_hold_full, 1);
    assert!(!snapshot.waiting_for_bbmd);
}

/// An address written while the device waits for a name ends the wait at
/// once.
#[tokio::test(start_paused = true)]
async fn an_address_written_while_waiting_ends_the_wait() {
    let named = ForeignBbmd::Name {
        host: NAME.into(),
        port: PORT,
    };
    let device = Device::configured(named, 60).await;
    device.after(1).await;
    assert!(device.sink.take().is_empty());
    device.activate(BACnetHostAddress::Ip(C.into())).unwrap();
    device.after(1).await;
    assert_eq!(registrations(&device.sink.take()), [C]);
    assert_eq!(device.bbmd(), at(C));
}

/// The configured name `bbmd.test`, which the table fails until told
/// otherwise.
fn configured_name() -> ForeignBbmd {
    ForeignBbmd::Name {
        host: NAME.into(),
        port: PORT,
    }
}

/// A name written while the retry of a configured name waits, which fails
/// too, sets up the next retry 30 s after its own failure: lookups go on
/// every 30 s, and the device registers once the name resolves (#2308).
#[tokio::test(start_paused = true)]
async fn a_name_written_while_its_retry_waits_keeps_the_retries_going() {
    let device = Device::configured(configured_name(), 60).await;
    device.after(1).await;
    assert_eq!(device.names.calls(), 1);
    device.after(10_000).await;
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    assert_eq!(device.names.calls(), 2);
    // 30 s after the newest failure, and every 30 s from then on.
    device.after(30_000).await;
    assert_eq!(device.names.calls(), 3);
    device.names.answer(NAME, Answer::addresses([A]));
    device.after(30_000).await;
    assert_eq!(device.names.calls(), 4);
    assert_eq!(registrations(&device.sink.take()), [A]);
    assert_eq!(device.bbmd(), at(A));
}

/// A stop and a start while the retry of a configured name waits: the
/// start looks the name up at once, and when that fails too, the next
/// retry comes 30 s after it, and so on until the name resolves (#2308).
#[tokio::test(start_paused = true)]
async fn a_restart_while_its_retry_waits_keeps_the_retries_going() {
    let mut device = Device::configured(configured_name(), 60).await;
    device.after(1).await;
    assert_eq!(device.names.calls(), 1);
    device.after(10_000).await;
    device.restart().await;
    device.after(1).await;
    assert_eq!(device.names.calls(), 2);
    device.after(30_000).await;
    assert_eq!(device.names.calls(), 3);
    device.names.answer(NAME, Answer::addresses([A]));
    device.after(30_000).await;
    assert_eq!(device.names.calls(), 4);
    assert_eq!(registrations(&device.sink.take()), [A]);
    assert_eq!(device.bbmd(), at(A));
}

/// The transport's stop cancels the retry of a configured name, rather
/// than leaving it to wake and find nothing to do: no lookup follows.
#[tokio::test(start_paused = true)]
async fn a_stop_cancels_the_retry() {
    let names = Arc::new(Names::default());
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd: configured_name(),
        ttl: 60,
        renewal_interval: None,
    });
    transport.set_bbmd_resolver(Arc::clone(&names) as Arc<dyn BbmdResolver>);
    let _rx = transport.start().await.unwrap();
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(names.calls(), 1);
    let target = Arc::clone(transport.foreign_device.as_ref().unwrap());
    assert!(target.lock().retry.is_some());
    transport.stop().await.unwrap();
    assert!(target.lock().retry.is_none());
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert_eq!(names.calls(), 1);
}

/// A name written while the device waits for a configured one is the
/// registration in effect from then on: `foreign_device_config` and the
/// next start read it back (#2308).
#[tokio::test(start_paused = true)]
async fn a_name_written_while_waiting_is_the_registration_in_effect() {
    let device = Device::configured(configured_name(), 60).await;
    device.after(1).await;
    let other = BACnetHostAddress::Name("other.test".into());
    device.activate(other).unwrap();
    device.after(1).await;
    let named = ForeignBbmd::Name {
        host: "other.test".into(),
        port: PORT,
    };
    assert_eq!(device.target.config().bbmd, named);
    assert_eq!(device.names.calls(), 2);
}
