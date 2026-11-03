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
use std::net::SocketAddrV4;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_transport::bbmd_lookup::BbmdLookupFailure;
#[cfg(feature = "tui")]
use bacnet_transport::bip::DEFAULT_BACNET_PORT;
use bacnet_transport::bip::{
    BipTransport, ForeignBbmd, ForeignDeviceConfig, ForeignRegistrationOutcome,
    ForeignRegistrationSnapshot,
};

/// How long a one-shot command waits for the first registration's answer.
/// The transport gives an attempt up after at most three seconds, so this
/// is a backstop for an attempt that never resolves.
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

/// Parse a `--foreign-bbmd` BBMD: `ip`, `host` or either with `:port`
/// (47808 by default). A host name is looked up when the transport starts
/// (#2308).
pub(crate) fn parse_foreign_bbmd_arg(value: &str) -> Result<ForeignBbmd, String> {
    value.parse::<ForeignBbmd>().map_err(|_| {
        format!("'{value}' is not a BBMD a device can register with: ip, host or either with :port")
    })
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
    let ended = registration.bbmd_lookups;
    let plural = if ended == 1 { "" } else { "s" };
    (ended > 0).then(|| {
        format!(
            "{ended} name lookup{plural}, {} failed",
            registration.bbmd_lookup_failures
        )
    })
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

    use super::{await_registration, config, lookup_failure, parse_foreign_bbmd_arg};

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
}
