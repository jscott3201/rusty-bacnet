//! FD_BBMD_Address naming the BBMD by host name (#1852): looked up at
//! activation and before each registration attempt, with failures counted
//! and the last good address kept. A foreign device on a recording socket,
//! under paused time, with a table for a resolver.

use std::collections::HashMap;
use std::io;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};

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

const OWN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), PORT);
const BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), PORT);
const PORT: u16 = 0xBAC0;
const A: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 1);
const B: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 2);
const C: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 1);
const NAME: &str = "bbmd.test";

/// What the table answers for a name.
#[derive(Clone)]
enum Answer {
    Addresses(Vec<Ipv4Addr>),
    Fails(io::ErrorKind),
    Hangs,
    /// These addresses, once the gate opens.
    Gated(Arc<Notify>, Vec<Ipv4Addr>),
}

/// A resolver that answers from a table and counts its lookups.
#[derive(Default)]
struct Names {
    answers: StdMutex<HashMap<String, Answer>>,
    calls: AtomicUsize,
}

impl Names {
    fn answer(&self, host: &str, answer: Answer) {
        self.answers.lock().unwrap().insert(host.into(), answer);
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl BbmdResolver for Names {
    fn resolve(&self, host: &str) -> BbmdLookup {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = self.answers.lock().unwrap().get(host).cloned();
        Box::pin(async move {
            match answer.unwrap_or(Answer::Fails(io::ErrorKind::NotFound)) {
                Answer::Addresses(addresses) => Ok(addresses),
                Answer::Fails(kind) => Err(io::Error::from(kind)),
                Answer::Hangs => std::future::pending().await,
                Answer::Gated(gate, addresses) => {
                    gate.notified().await;
                    Ok(addresses)
                }
            }
        })
    }
}

/// A foreign device at [`OWN`] registering with [`A`], never answered, so
/// it retries: each attempt times out after 3 s, and the next comes by the
/// renewal interval, half the time-to-live.
struct Device {
    target: Arc<ForeignTarget>,
    node: Arc<NodeHandle>,
    sink: SendSink,
    names: Arc<Names>,
    _driver: JoinHandle<()>,
}

impl Device {
    async fn new(ttl: u16) -> Self {
        let sink = SendSink::default();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let socket = Arc::new(BipSocket::recording(socket, sink.clone()));
        let config = ForeignDeviceConfig {
            bbmd_ip: A,
            bbmd_port: PORT,
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
            _driver: driver,
        }
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
        self.target.config().bbmd()
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
    device.names.answer(NAME, Answer::Addresses(vec![B]));
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

/// Each registration attempt looks the name up again. While the address in
/// use is among the answers the device stays; once it isn't, the device
/// moves, and the attempt that move starts looks nothing up.
#[tokio::test(start_paused = true)]
async fn each_registration_attempt_looks_the_name_up_again() {
    let device = Device::new(10).await;
    device.after(1).await;
    device.sink.take();
    device.names.answer(NAME, Answer::Addresses(vec![C, A]));
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    assert!(registrations(&device.sink.take()).is_empty());
    assert_eq!((device.names.calls(), device.bbmd()), (1, at(A)));

    assert_eq!(registrations(&device.next_attempt().await), [A]);
    assert_eq!((device.names.calls(), device.bbmd()), (2, at(A)));

    device.names.answer(NAME, Answer::Addresses(vec![C]));
    assert_eq!(registrations(&device.next_attempt().await), [A, C]);
    assert_eq!((device.names.calls(), device.bbmd()), (3, at(C)));

    assert_eq!(registrations(&device.next_attempt().await), [C]);
    assert_eq!((device.names.calls(), device.bbmd()), (4, at(C)));
    let snapshot = device.snapshot();
    assert_eq!(
        (snapshot.bbmd_lookups, snapshot.bbmd_lookup_failures),
        (4, 0)
    );
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
        (Answer::Addresses(vec![]), BbmdLookupFailure::NoIpv4Address),
        // Its own address, then a broadcast and the unspecified address,
        // as a start would refuse them.
        (
            Answer::Addresses(vec![*OWN.ip()]),
            BbmdLookupFailure::Unusable(*OWN.ip()),
        ),
        (
            Answer::Addresses(vec![*BROADCAST.ip(), Ipv4Addr::UNSPECIFIED]),
            BbmdLookupFailure::Unusable(*BROADCAST.ip()),
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

    device.names.answer(NAME, Answer::Addresses(vec![B]));
    // The attempt made while the lookup hung may come first.
    let mut seen = Vec::new();
    for _ in 0..3 {
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

/// A lookup started for a name that an address, or another name, has since
/// replaced changes nothing when it ends, and counts nothing; with an
/// address, attempts look nothing up.
#[tokio::test(start_paused = true)]
async fn a_lookup_for_a_replaced_name_changes_nothing() {
    let device = Device::new(10).await;
    device.after(1).await;
    device.sink.take();
    let gate = Arc::new(Notify::new());
    device
        .names
        .answer(NAME, Answer::Gated(Arc::clone(&gate), vec![B]));
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

    // Replaced by another name, which resolves first: the first name's late
    // answer doesn't move the device back.
    let gate = Arc::new(Notify::new());
    device
        .names
        .answer(NAME, Answer::Gated(Arc::clone(&gate), vec![B]));
    device
        .names
        .answer("other.test", Answer::Addresses(vec![A]));
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    let other = BACnetHostAddress::Name("other.test".into());
    device.activate(other.clone()).unwrap();
    device.after(1).await;
    assert_eq!(device.bbmd(), at(A));
    gate.notify_one();
    device.after(1).await;
    assert_eq!(device.bbmd(), at(A));
    assert_eq!(
        device.view().bbmd(),
        BACnetHostNPort {
            host: other,
            port: PORT
        }
    );
    assert_eq!(registrations(&device.sink.take()), [A]);
    assert_eq!(device.snapshot().bbmd_lookups, 1);
}

/// A lookup that ends once the node has stopped changes and counts
/// nothing: a start looks the name up again.
#[tokio::test(start_paused = true)]
async fn a_lookup_ending_after_a_stop_counts_nothing() {
    let device = Device::new(60).await;
    device.after(1).await;
    let gate = Arc::new(Notify::new());
    device
        .names
        .answer(NAME, Answer::Gated(Arc::clone(&gate), vec![B]));
    device.activate(Device::name()).unwrap();
    device.after(1).await;
    device.node.stop();
    gate.notify_one();
    device.after(1).await;
    assert_eq!(device.bbmd(), at(A));
    assert_eq!(device.snapshot().bbmd_lookups, 0);
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
