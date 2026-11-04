//! FD_BBMD_Address naming a B/IPv6 BBMD by host name (#2308): looked up at
//! activation and alongside the re-registrations, on B/IP's policy
//! (`crate::bbmd_lookup`), keeping the IPv6 answers. The target alone, on
//! loopback (`::1`), under paused time, with a table for a resolver; no
//! datagram is sent. `foreign_retarget_tests.rs` has the transport ones.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV6};
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::bip_port::{ForeignDeviceChanges, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, RemoteError};
use tokio::sync::Notify;

use super::super::port::Bip6ForeignDeviceConfig;
use super::ForeignTarget;
use crate::bbmd_lookup::table::{Answer, Names};
use crate::bbmd_lookup::{BbmdLookupCounters, BbmdLookupFailure, BbmdResolver};

const NAME: &str = "bbmd.test";
/// The BBMD it starts with, the one a name moves it to, and its own port.
const A: u16 = 47_901;
const B: u16 = 47_902;
const OWN: u16 = 47_900;

fn at(port: u16) -> SocketAddrV6 {
    SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0)
}

/// A B/IPv6 foreign device's target at `[::1]:OWN`, registered with
/// `[::1]:A` for 60 s, its transport running.
struct Device {
    target: Arc<ForeignTarget>,
    names: Arc<Names>,
}

impl Device {
    fn new() -> Self {
        let names = Arc::new(Names::default());
        let config = Bip6ForeignDeviceConfig {
            bbmd: (Ipv6Addr::LOCALHOST, A).into(),
            ttl: 60,
        };
        let resolver = Arc::clone(&names) as Arc<dyn BbmdResolver>;
        let target = ForeignTarget::new(config, resolver);
        target.attach(at(OWN), None);
        Self { target, names }
    }

    /// Activate FD_BBMD_Address as `host` at `port`.
    fn activate(&self, host: BACnetHostAddress, port: u16) -> Result<(), Error> {
        let mut changes = ForeignDeviceChanges::default();
        changes.bbmd = Some(BACnetHostNPort { host, port });
        self.target.activate(&changes)
    }

    fn name() -> BACnetHostAddress {
        BACnetHostAddress::Name(NAME.into())
    }

    /// Let the paused clock run `ms` milliseconds, and the lookups with it.
    async fn after(&self, ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    fn bbmd(&self) -> SocketAddrV6 {
        self.target.bbmd_addr().expect("a BBMD address")
    }

    fn counters(&self) -> BbmdLookupCounters {
        self.target.lookup_counters()
    }

    /// Whether the re-registration task was woken since the last look.
    async fn woken(&self) -> bool {
        tokio::time::timeout(Duration::from_millis(1), self.target.changed())
            .await
            .is_ok()
    }
}

/// Activated, the name is looked up and the device moves to the first IPv6
/// address it stands for, an IPv4 one aside; the re-registration task is
/// woken for it once, after the lookup, and the row reads back the name.
#[tokio::test(start_paused = true)]
async fn a_name_is_looked_up_at_activation_and_the_device_moves_there() {
    let device = Device::new();
    let answers = [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ];
    let gate = Arc::new(Notify::new());
    device.names.answer(NAME, Answer::gated(&gate, answers));
    device.activate(Device::name(), B).unwrap();
    device.after(1).await;
    // Nothing moves, or registers, before the answer.
    assert_eq!(device.bbmd(), at(A));
    assert!(!device.woken().await);
    gate.notify_one();
    device.after(1).await;
    assert_eq!(device.bbmd(), at(B));
    assert!(device.woken().await);
    assert_eq!(
        ForeignRegistration::bbmd(&*device.target),
        BACnetHostNPort {
            host: Device::name(),
            port: B
        }
    );
    let counters = device.counters();
    assert_eq!((counters.lookups, counters.failures), (1, 0));
    assert_eq!(counters.last_failure, None);
}

/// Later lookups run alongside the re-registrations, at most once per 30 s;
/// while the address in use is among the answers the device stays, and once
/// it isn't, it moves.
#[tokio::test(start_paused = true)]
async fn attempts_look_the_name_up_again_at_most_once_per_30_s() {
    let device = Device::new();
    device
        .names
        .answer(NAME, Answer::addresses([Ipv6Addr::LOCALHOST]));
    device.activate(Device::name(), A).unwrap();
    device.after(1).await;
    assert_eq!((device.names.calls(), device.bbmd()), (1, at(A)));
    device.target.attempting();
    device.after(29_000).await;
    device.target.attempting();
    device.after(1).await;
    assert_eq!(device.names.calls(), 1);
    device.after(1_000).await;
    device.target.attempting();
    device.after(1).await;
    assert_eq!((device.names.calls(), device.bbmd()), (2, at(A)));
    // A start looks it up at once, whenever the last lookup was.
    device.activate(Device::name(), B).unwrap();
    device.after(1).await;
    assert_eq!((device.names.calls(), device.bbmd()), (3, at(B)));
    device.target.detach();
    device.target.attach(at(OWN), None);
    device.target.attempting();
    device.after(1).await;
    assert_eq!(device.names.calls(), 4);
}

/// A lookup that fails, finds no IPv6 address, or only IPv6 addresses the
/// device can't register with is counted with its reason, and the device
/// keeps registering where it was. So does one that hangs, after 10 s.
#[tokio::test(start_paused = true)]
async fn a_failed_lookup_is_counted_and_the_device_stays_where_it_was() {
    let device = Device::new();
    let parse = |text: &str| text.parse::<Ipv6Addr>().unwrap();
    let unusable = |text: &str| BbmdLookupFailure::Unusable(IpAddr::V6(parse(text)));
    let cases = [
        (
            Answer::Fails(io::ErrorKind::NotFound),
            BbmdLookupFailure::Error(io::ErrorKind::NotFound),
        ),
        (
            Answer::addresses([Ipv4Addr::LOCALHOST]),
            BbmdLookupFailure::NoAddress,
        ),
        // A link-local, an IPv4-mapped and a multicast address, its own
        // address, and one the bound socket can't reach: a Network Port
        // write would refuse each.
        (Answer::addresses([parse("fe80::1")]), unusable("fe80::1")),
        (
            Answer::addresses([parse("::ffff:10.0.0.1")]),
            unusable("::ffff:10.0.0.1"),
        ),
        (
            Answer::addresses([parse("ff05::bac0")]),
            unusable("ff05::bac0"),
        ),
        (
            Answer::addresses([parse("2001:db8::1")]),
            unusable("2001:db8::1"),
        ),
    ];
    let count = cases.len() as u64 + 2;
    for (answer, failure) in cases {
        device.names.answer(NAME, answer);
        device.activate(Device::name(), B).unwrap();
        device.after(1).await;
        assert_eq!(device.counters().last_failure, Some(failure));
        assert_eq!(device.bbmd(), at(A), "{failure:?}");
    }
    device
        .names
        .answer(NAME, Answer::addresses([Ipv6Addr::LOCALHOST]));
    device.activate(Device::name(), OWN).unwrap();
    device.after(1).await;
    assert_eq!(
        device.counters().last_failure,
        Some(BbmdLookupFailure::Unusable(Ipv6Addr::LOCALHOST.into()))
    );
    device.names.answer(NAME, Answer::Hangs);
    device.activate(Device::name(), B).unwrap();
    device.after(10_001).await;
    let counters = device.counters();
    assert_eq!(counters.last_failure, Some(BbmdLookupFailure::TimedOut));
    assert_eq!((counters.lookups, counters.failures), (count, count));
    assert_eq!(device.bbmd(), at(A));
    assert!(!device.woken().await);

    // The next good answer moves it, and clears the reason.
    device
        .names
        .answer(NAME, Answer::addresses([Ipv6Addr::LOCALHOST]));
    device.activate(Device::name(), B).unwrap();
    device.after(1).await;
    assert_eq!(device.bbmd(), at(B));
    assert_eq!(device.counters().last_failure, None);
}

/// A lookup that ends with the transport stopped changes and counts
/// nothing; a stop also clears the newest failure.
#[tokio::test(start_paused = true)]
async fn a_lookup_ending_after_a_stop_counts_nothing() {
    let device = Device::new();
    let gate = Arc::new(Notify::new());
    device
        .names
        .answer(NAME, Answer::gated(&gate, [Ipv6Addr::LOCALHOST]));
    device.activate(Device::name(), B).unwrap();
    device.after(1).await;
    device.target.detach();
    gate.notify_one();
    device.after(1).await;
    assert_eq!(device.bbmd(), at(A));
    assert_eq!(device.counters(), BbmdLookupCounters::default());
}

/// A written name is checked for its form and port, and the check changes
/// nothing and looks nothing up; an address in the row ends the name.
#[tokio::test(start_paused = true)]
async fn a_written_name_is_checked_for_its_form() {
    let device = Device::new();
    let written = |host: &str, port| {
        let mut changes = ForeignDeviceChanges::default();
        changes.bbmd = Some(BACnetHostNPort {
            host: BACnetHostAddress::Name(host.into()),
            port,
        });
        changes
    };
    for (host, port) in [("bbmd test", B), ("-bbmd", B), ("", B), (NAME, 0)] {
        for error in [
            device.target.check(&written(host, port)).unwrap_err(),
            device.target.activate(&written(host, port)).unwrap_err(),
        ] {
            assert!(
                matches!(error, Error::Remote(RemoteError::Error { class, code })
                    if class == ErrorClass::PROPERTY && code == ErrorCode::VALUE_OUT_OF_RANGE),
                "{host:?}:{port}: {error:?}"
            );
        }
    }
    device.target.check(&written(NAME, B)).unwrap();
    device.after(1).await;
    assert_eq!(device.names.calls(), 0);

    device
        .names
        .answer(NAME, Answer::addresses([Ipv6Addr::LOCALHOST]));
    device.activate(Device::name(), A).unwrap();
    device
        .activate(BACnetHostAddress::Ip(Ipv6Addr::LOCALHOST.into()), B)
        .unwrap();
    device.after(1).await;
    assert_eq!(device.bbmd(), at(B));
    assert_eq!(
        ForeignRegistration::bbmd(&*device.target),
        BACnetHostNPort::from_socket_addr(at(B).into())
    );
}
