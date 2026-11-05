//! `--foreign-bbmd`: run the CLI's own B/IP transport as a foreign device
//! (#938), so its broadcasts reach a remote subnet through that BBMD.
//!
//! The transport registers at start and renews on its own. A one-shot
//! command waits for the first registration to be answered before it runs,
//! and fails when the BBMD refuses it or doesn't answer, or when its host
//! name doesn't resolve (#2308); the TUI reports the outcome and carries
//! on.

#[cfg(feature = "tui")]
use std::net::Ipv4Addr;
use std::net::{SocketAddrV4, SocketAddrV6};
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_transport::bbmd_lookup::{BbmdLookupCounters, BbmdLookupFailure};
#[cfg(feature = "tui")]
use bacnet_transport::bip::DEFAULT_BACNET_PORT;
use bacnet_transport::bip::{
    BipTransport, ForeignBbmd, ForeignDeviceConfig, ForeignRegistrationOutcome,
    ForeignRegistrationSnapshot,
};
use bacnet_transport::bip6::{
    Bip6ForeignBbmd, Bip6RegistrationAnswer, Bip6Transport, Bvlc6ResultCode,
};

/// How long a one-shot command waits for the first registration's answer.
/// The B/IP transport gives an attempt up after at most three seconds, so
/// this is a backstop for an attempt that never resolves; B/IPv6 gives up
/// on none, so this is how long its BBMD gets (#2388).
const REGISTRATION_WAIT: Duration = Duration::from_secs(5);

/// Parse the TUI's `bbmd` address, the BBMD whose tables it reads: `ip` or
/// `ip:port` (47808 by default).
#[cfg(feature = "tui")]
pub(crate) fn parse_bbmd_arg(value: &str) -> Result<SocketAddrV4, String> {
    let address = match value.parse::<SocketAddrV4>() {
        Ok(address) => address,
        Err(_) => {
            let ip = value
                .parse::<Ipv4Addr>()
                .map_err(|_| format!("'{value}' is not an IPv4 address or ip:port"))?;
            SocketAddrV4::new(ip, DEFAULT_BACNET_PORT)
        }
    };
    if address.port() == 0 || address.ip().is_unspecified() {
        return Err(format!(
            "'{value}' is not a BBMD a device can register with"
        ));
    }
    Ok(address)
}

/// Parse a B/IP BBMD: `ip`, `host` or either with `:port` (47808 by
/// default), as a TUI profile gives one. A host name is looked up when the
/// transport starts (#2308).
#[cfg(any(feature = "tui", test))]
pub(crate) fn parse_foreign_bbmd_arg(value: &str) -> Result<ForeignBbmd, String> {
    value.parse::<ForeignBbmd>().map_err(|_| {
        format!("'{value}' is not a BBMD a device can register with: ip, host or either with :port")
    })
}

/// `--foreign-bbmd`: a B/IP BBMD (an IPv4 address or a host name) or a
/// B/IPv6 address. A host name serves either transport; the session's
/// transport takes it (#2365).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ForeignBbmdArg {
    /// An IPv4 address or a host name, with its port.
    Bip(ForeignBbmd),
    /// An IPv6 address, with its port.
    Bip6(SocketAddrV6),
}

impl ForeignBbmdArg {
    /// The BBMD for a B/IP session.
    pub(crate) fn bip(&self) -> Result<ForeignBbmd, String> {
        match self {
            Self::Bip(bbmd) => Ok(bbmd.clone()),
            Self::Bip6(bbmd) => Err(format!(
                "--foreign-bbmd {bbmd} is a B/IPv6 BBMD: add --ipv6, or give an IPv4 address"
            )),
        }
    }

    /// The BBMD for a B/IPv6 session.
    pub(crate) fn bip6(&self) -> Result<Bip6ForeignBbmd, String> {
        match self {
            Self::Bip6(bbmd) => Ok((*bbmd).into()),
            Self::Bip(ForeignBbmd::Name { host, port }) => Ok(Bip6ForeignBbmd::Name {
                host: host.clone(),
                port: *port,
            }),
            Self::Bip(bbmd) => Err(format!(
                "--foreign-bbmd {bbmd} is an IPv4 address, which B/IPv6 can't register with: \
                 give an IPv6 address ('[ip]:port') or a host name"
            )),
        }
    }
}

/// Parse `--foreign-bbmd`: `ip`, `host` or either with `:port` for B/IP,
/// or `ip`, `[ip]` or `[ip]:port` for B/IPv6 (47808 by default).
pub(crate) fn parse_foreign_bbmd_cli_arg(value: &str) -> Result<ForeignBbmdArg, String> {
    if let Ok(bbmd) = value.parse::<ForeignBbmd>() {
        return Ok(ForeignBbmdArg::Bip(bbmd));
    }
    match value.parse::<Bip6ForeignBbmd>() {
        Ok(Bip6ForeignBbmd::Address(address)) => Ok(ForeignBbmdArg::Bip6(address)),
        _ => Err(format!(
            "'{value}' is not a BBMD a device can register with: ip, host or either with :port, \
             or an IPv6 address as ip or [ip]:port"
        )),
    }
}

/// Parse a `--nat-global-address` (#1943): the `ip:port` a NAT router
/// gives our registration, as the BBMD records it. The port is required,
/// since a NAT router seldom keeps ours.
pub(crate) fn parse_nat_global_arg(value: &str) -> Result<SocketAddrV4, String> {
    let address = value
        .parse::<SocketAddrV4>()
        .map_err(|_| "not an IPv4 ip:port".to_string())?;
    nat_global_address(address).map_err(|why| format!("it {why}"))
}

/// Check a NAT global address: an IPv4 unicast endpoint with a nonzero
/// port, the only kind an FDT entry can hold. `Err` says why not.
pub(crate) fn nat_global_address(address: SocketAddrV4) -> Result<SocketAddrV4, &'static str> {
    let ip = address.ip();
    if address.port() == 0 {
        Err("needs a nonzero port")
    } else if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
        Err("is not a unicast address a BBMD could record")
    } else {
        Ok(address)
    }
}

/// The foreign-device configuration the flags ask for, if any.
pub(crate) fn config(
    bbmd: Option<ForeignBbmd>,
    ttl: u16,
    renewal_secs: Option<u64>,
) -> Option<ForeignDeviceConfig> {
    bbmd.map(|bbmd| ForeignDeviceConfig {
        bbmd,
        ttl,
        renewal_interval: renewal_secs.map(Duration::from_secs),
    })
}

/// Wait until the first registration with `bbmd` is answered: `Ok`, naming
/// the BBMD, once it accepts it, an error naming the BBMD and the outcome
/// otherwise. A BBMD named by a host name is named with the address it
/// stands for and its lookups (#2308); one whose name doesn't resolve
/// fails at once with the lookup's reason: the transport tries it again
/// every 30 seconds, after this wait is over.
pub(crate) async fn await_registration(
    client: &BACnetClient<BipTransport>,
    bbmd: &ForeignBbmd,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + REGISTRATION_WAIT;
    loop {
        let registration = client.foreign_registration().map_err(|e| e.to_string())?;
        if registration.last_outcome.is_none()
            && let Some(failure) = registration.last_bbmd_lookup_failure
        {
            let tally = lookups(&registration).map_or_else(String::new, |n| format!(" ({n})"));
            return Err(format!(
                "BBMD {bbmd} could not be registered with: its host name {}{tally}",
                lookup_failure(failure)
            ));
        }
        let outcome = registration.last_outcome;
        let pending = matches!(outcome, Some(ForeignRegistrationOutcome::Pending) | None);
        if pending && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        }
        let bbmd = label(bbmd, &registration);
        return match outcome {
            Some(ForeignRegistrationOutcome::Accepted) => Ok(bbmd),
            Some(ForeignRegistrationOutcome::Refused) => {
                let result = registration
                    .last_result
                    .map_or_else(String::new, |result| format!(" ({result})"));
                Err(format!(
                    "BBMD {bbmd} refused the foreign-device registration{result}"
                ))
            }
            Some(ForeignRegistrationOutcome::LocalError) => Err(format!(
                "could not send the foreign-device registration to BBMD {bbmd}"
            )),
            Some(ForeignRegistrationOutcome::TimedOut | ForeignRegistrationOutcome::Cancelled) => {
                Err(format!(
                    "BBMD {bbmd} did not answer the foreign-device registration"
                ))
            }
            Some(ForeignRegistrationOutcome::Pending) | None => Err(format!(
                "the foreign-device registration with BBMD {bbmd} had no outcome after {} s",
                REGISTRATION_WAIT.as_secs()
            )),
        };
    }
}

/// `bbmd` as the messages name it: a host name together with the address
/// it stands for, once an attempt went there, and its lookups (#2308).
fn label(bbmd: &ForeignBbmd, registration: &ForeignRegistrationSnapshot) -> String {
    if bbmd.address().is_some() {
        return bbmd.to_string();
    }
    let at = registration.last_bbmd.map(|at| format!("at {at}"));
    let parts: Vec<String> = at.into_iter().chain(lookups(registration)).collect();
    if parts.is_empty() {
        return bbmd.to_string();
    }
    format!("{bbmd} ({})", parts.join("; "))
}

/// How many lookups of the BBMD's host name ended, and how many of them
/// failed (#2308), for a person to read; `None` before the first ends.
pub(crate) fn lookups(registration: &ForeignRegistrationSnapshot) -> Option<String> {
    tally(registration.bbmd_lookups, registration.bbmd_lookup_failures)
}

/// `ended` lookups, `failed` of them failed, for a person to read; `None`
/// before the first ends.
fn tally(ended: u64, failed: u64) -> Option<String> {
    let plural = if ended == 1 { "" } else { "s" };
    (ended > 0).then(|| format!("{ended} name lookup{plural}, {failed} failed"))
}

/// How long a one-shot command waits for a B/IPv6 BBMD's host name: the
/// resolver's 10 s, and a little more.
const BIP6_LOOKUP_WAIT: Duration = Duration::from_secs(11);

/// Wait until a B/IPv6 foreign device's BBMD answers its first
/// registration, as [`await_registration`] waits on B/IP: `Ok`, naming the
/// BBMD, once the BBMD holds it (#2388), an error naming the BBMD and the
/// outcome otherwise. A BBMD named by a host name is waited for first, at
/// most [`BIP6_LOOKUP_WAIT`], and named with the address it stands for and
/// its lookups; one whose name doesn't resolve fails at once with the
/// lookup's reason (#2365).
pub(crate) async fn await_bip6_registration(
    client: &BACnetClient<Bip6Transport>,
    bbmd: &Bip6ForeignBbmd,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + BIP6_LOOKUP_WAIT;
    // When the registration's answer is given up on, once one went.
    let mut answer_by = None;
    loop {
        let status = client
            .transport()
            .foreign_status()
            .ok_or("the client is not a foreign device")?;
        let tally = bip6_lookups(&status.lookups);
        if let Some(at) = status.bbmd
            && status.registrations_sent > 0
        {
            let now = tokio::time::Instant::now();
            let by = *answer_by.get_or_insert(now + REGISTRATION_WAIT);
            let named = if bbmd.address().is_some() {
                bbmd.to_string()
            } else {
                let tally = tally.map_or_else(String::new, |tally| format!("; {tally}"));
                format!("{bbmd} (at {at}{tally})")
            };
            match status.last_answer {
                Some(Bip6RegistrationAnswer::Held) => return Ok(named),
                Some(Bip6RegistrationAnswer::Refused) => {
                    return Err(format!(
                        "BBMD {named} refused the foreign-device registration ({})",
                        Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK
                    ));
                }
                Some(Bip6RegistrationAnswer::Unanswered) => {}
                // An answer this CLI doesn't name yet, or none so far.
                _ if now < by => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                }
                _ => {}
            }
            return Err(format!(
                "BBMD {named} did not answer the foreign-device registration"
            ));
        }
        if let Some(failure) = status.lookups.last_failure {
            let tally = tally.map_or_else(String::new, |tally| format!(" ({tally})"));
            return Err(format!(
                "BBMD {bbmd} could not be registered with: its host name {}{tally}",
                bip6_lookup_failure(failure)
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            let wait = BIP6_LOOKUP_WAIT.as_secs();
            return Err(match status.bbmd {
                Some(at) => {
                    format!("no registration went to BBMD {bbmd} (at {at}) within {wait} s")
                }
                None => {
                    format!("the lookup of BBMD {bbmd}'s host name had no outcome after {wait} s")
                }
            });
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// How many lookups of a B/IPv6 BBMD's host name ended, and how many of
/// them failed, as [`lookups`] says them for B/IP; `None` before the first
/// ends.
pub(crate) fn bip6_lookups(lookups: &BbmdLookupCounters) -> Option<String> {
    tally(lookups.lookups, lookups.failures)
}

/// Why a lookup of a B/IPv6 BBMD's host name failed (#2365): as
/// [`lookup_failure`] says it for B/IP, except that the name must stand
/// for an IPv6 address.
pub(crate) fn bip6_lookup_failure(failure: BbmdLookupFailure) -> String {
    match failure {
        BbmdLookupFailure::NoAddress => "has no IPv6 address".into(),
        other => lookup_failure(other),
    }
}

/// Why a lookup of the BBMD's host name failed, for a person to read.
pub(crate) fn lookup_failure(failure: BbmdLookupFailure) -> String {
    match failure {
        // The kind is all a snapshot keeps. Only these say something more;
        // a name service's own failure arrives uncategorized, a kind stable
        // Rust can't name, so every other kind is left out.
        BbmdLookupFailure::Error(kind) => {
            use std::io::ErrorKind::{
                ConnectionRefused, HostUnreachable, InvalidInput, NetworkDown, NetworkUnreachable,
                NotFound, PermissionDenied, TimedOut,
            };
            match kind {
                NotFound | PermissionDenied | ConnectionRefused | TimedOut | InvalidInput
                | HostUnreachable | NetworkUnreachable | NetworkDown => {
                    format!("did not resolve ({kind})")
                }
                _ => "did not resolve".into(),
            }
        }
        BbmdLookupFailure::TimedOut => "had no answer from the resolver within 10 s".into(),
        BbmdLookupFailure::NoAddress => "has no IPv4 address".into(),
        BbmdLookupFailure::Unusable(ip) => {
            format!("resolves only to addresses a device can't register with, such as {ip}")
        }
        BbmdLookupFailure::Abandoned => "lookup ended without an answer".into(),
        other => format!("lookup failed ({other:?})"),
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::net::Ipv4Addr;
    use std::sync::Arc;
    use std::time::Duration;

    use bacnet_client::client::BACnetClient;
    use bacnet_transport::bbmd_lookup::{BbmdLookup, BbmdLookupFailure, BbmdResolver};
    use bacnet_transport::bip::BipMode;

    use super::{
        await_bip6_registration, await_registration, config, lookup_failure, parse_foreign_bbmd_arg,
    };

    #[test]
    fn a_lookup_failure_reads_as_a_reason() {
        for (failure, text) in [
            (
                BbmdLookupFailure::Error(io::ErrorKind::NotFound),
                "did not resolve (entity not found)",
            ),
            (BbmdLookupFailure::TimedOut, "within 10 s"),
            (BbmdLookupFailure::NoAddress, "no IPv4 address"),
            (
                BbmdLookupFailure::Unusable(Ipv4Addr::BROADCAST.into()),
                "255.255.255.255",
            ),
            (BbmdLookupFailure::Abandoned, "without an answer"),
        ] {
            assert!(lookup_failure(failure).contains(text), "{failure:?}");
        }
        // A kind that says nothing more is left out.
        let other = BbmdLookupFailure::Error(io::ErrorKind::Other);
        assert_eq!(lookup_failure(other), "did not resolve");
    }

    /// A resolver that knows no name.
    struct NoNames;

    impl BbmdResolver for NoNames {
        fn resolve(&self, _host: &str) -> BbmdLookup {
            Box::pin(async { Err(io::Error::from(io::ErrorKind::NotFound)) })
        }
    }

    /// A BBMD named by a host name that doesn't resolve still starts the
    /// client (#2308); waiting for its registration then fails at once,
    /// naming the BBMD and why its name didn't resolve, rather than waiting
    /// out a registration that never went.
    #[tokio::test]
    async fn a_name_that_does_not_resolve_starts_and_fails_the_wait() {
        let bbmd = parse_foreign_bbmd_arg("bbmd.invalid").unwrap();
        let mode = BipMode::Foreign(config(Some(bbmd.clone()), 60, None).unwrap());
        let mut client = BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .mode(mode)
            .bbmd_resolver(Arc::new(NoNames))
            .build()
            .await
            .unwrap();
        let started = tokio::time::Instant::now();
        let error = await_registration(&client, &bbmd).await.unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(4), "{error}");
        assert!(error.contains("bbmd.invalid:47808"), "{error}");
        assert!(error.contains("host name did not resolve"), "{error}");
        assert!(error.ends_with("(1 name lookup, 1 failed)"), "{error}");
        assert_eq!(client.foreign_registration().unwrap().attempts, 0);
        client.stop().await.unwrap();
    }

    /// A resolver that stands every name for 127.0.0.1.
    struct Loopback;

    impl BbmdResolver for Loopback {
        fn resolve(&self, _host: &str) -> BbmdLookup {
            Box::pin(async { Ok(vec![Ipv4Addr::LOCALHOST.into()]) })
        }
    }

    /// A name that resolves to a BBMD that never answers: the error names
    /// the BBMD by its name, the address it stood for and its lookups
    /// (#2308).
    #[tokio::test(start_paused = true)]
    async fn a_resolved_name_is_reported_with_its_address_and_lookups() {
        let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = silent.local_addr().unwrap().port();
        let bbmd = parse_foreign_bbmd_arg(&format!("bbmd.example:{port}")).unwrap();
        let mode = BipMode::Foreign(config(Some(bbmd.clone()), 60, None).unwrap());
        let mut client = BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .mode(mode)
            .bbmd_resolver(Arc::new(Loopback))
            .build()
            .await
            .unwrap();
        let error = await_registration(&client, &bbmd).await.unwrap_err();
        assert_eq!(
            error,
            format!(
                "BBMD bbmd.example:{port} (at 127.0.0.1:{port}; 1 name lookup, 0 failed) \
                 did not answer the foreign-device registration"
            )
        );
        client.stop().await.unwrap();
    }

    /// A resolver that stands every name for the IPv6 loopback address.
    struct Loopback6;

    impl BbmdResolver for Loopback6 {
        fn resolve(&self, _host: &str) -> BbmdLookup {
            Box::pin(async { Ok(vec![std::net::Ipv6Addr::LOCALHOST.into()]) })
        }
    }

    /// A B/IPv6 client on `::1` registering with `bbmd`, its names looked
    /// up by `resolver`.
    async fn bip6_client(
        bbmd: &str,
        resolver: Arc<dyn BbmdResolver>,
    ) -> BACnetClient<bacnet_transport::bip6::Bip6Transport> {
        use bacnet_transport::bip6::Bip6ForeignDeviceConfig;
        BACnetClient::bip6_builder()
            .interface(std::net::Ipv6Addr::LOCALHOST)
            .port(0)
            .device_instance(4321)
            .foreign_device(Bip6ForeignDeviceConfig {
                bbmd: bbmd.parse().unwrap(),
                ttl: 60,
            })
            .bbmd_resolver(resolver)
            .build()
            .await
            .unwrap()
    }

    /// A B/IPv6 BBMD on `::1` that answers the first Register-Foreign-Device
    /// it gets with a BVLC-Result of `code`, and its port.
    async fn answering_bbmd(code: u16) -> u16 {
        let bbmd = tokio::net::UdpSocket::bind("[::1]:0").await.unwrap();
        let port = bbmd.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut buf = [0u8; 64];
            loop {
                let (len, from) = bbmd.recv_from(&mut buf).await.unwrap();
                // BVLL for B/IPv6, Register-Foreign-Device (Annex U.2.10).
                if buf[..len].starts_with(&[0x82, 0x09]) {
                    let [hi, lo] = code.to_be_bytes();
                    let result = [0x82, 0x00, 0x00, 0x09, 0x0B, 0xBB, 0xBB, hi, lo];
                    bbmd.send_to(&result, from).await.unwrap();
                    return;
                }
            }
        });
        port
    }

    /// A B/IPv6 one-shot waits for its BBMD's host name (#2365): one that
    /// doesn't resolve fails the wait with why and the lookups, and one
    /// that does names the address it stands for once the BBMD there takes
    /// the registration (#2388).
    #[tokio::test]
    async fn a_bip6_one_shot_waits_for_its_bbmds_name() {
        let mut client = bip6_client("bbmd.invalid", Arc::new(NoNames)).await;
        let bbmd = client.transport().foreign_device_config().unwrap().bbmd;
        let error = await_bip6_registration(&client, &bbmd).await.unwrap_err();
        assert!(
            error.starts_with("BBMD bbmd.invalid:47808 could not be registered with: its host name did not resolve"),
            "{error}"
        );
        assert!(error.ends_with("(1 name lookup, 1 failed)"), "{error}");
        client.stop().await.unwrap();

        let port = answering_bbmd(0x0000).await;
        let named = format!("bbmd.example:{port}");
        let mut client = bip6_client(&named, Arc::new(Loopback6)).await;
        let bbmd: bacnet_transport::bip6::Bip6ForeignBbmd = named.parse().unwrap();
        let found = await_bip6_registration(&client, &bbmd).await.unwrap();
        assert_eq!(
            found,
            format!("{named} (at [::1]:{port}; 1 name lookup, 0 failed)")
        );
        client.stop().await.unwrap();
    }

    /// A B/IPv6 BBMD's refusal fails the one-shot, as B/IP's does (#2388).
    #[tokio::test]
    async fn a_bip6_bbmds_refusal_fails_the_one_shot() {
        let port = answering_bbmd(0x0090).await;
        let at = format!("[::1]:{port}");
        let mut client = bip6_client(&at, Arc::new(NoNames)).await;
        let bbmd = client.transport().foreign_device_config().unwrap().bbmd;
        let error = await_bip6_registration(&client, &bbmd).await.unwrap_err();
        assert_eq!(
            error,
            format!(
                "BBMD {at} refused the foreign-device registration (REGISTER_FOREIGN_DEVICE_NAK)"
            )
        );
        client.stop().await.unwrap();
    }

    /// A B/IPv6 BBMD that never answers fails the one-shot after the wait,
    /// as B/IP's does (#2388).
    #[tokio::test(start_paused = true)]
    async fn a_silent_bip6_bbmd_fails_the_one_shot() {
        let silent = tokio::net::UdpSocket::bind("[::1]:0").await.unwrap();
        let at = format!("[::1]:{}", silent.local_addr().unwrap().port());
        let mut client = bip6_client(&at, Arc::new(NoNames)).await;
        let bbmd = client.transport().foreign_device_config().unwrap().bbmd;
        let error = await_bip6_registration(&client, &bbmd).await.unwrap_err();
        assert_eq!(
            error,
            format!("BBMD {at} did not answer the foreign-device registration")
        );
        client.stop().await.unwrap();
    }
}
