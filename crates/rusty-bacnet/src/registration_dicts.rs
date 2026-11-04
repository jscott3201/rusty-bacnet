//! The dicts `foreign_registration()` returns: a B/IP foreign device's
//! registration (#938), or a B/IPv6 one's (#2365).

use std::time::Duration;

use bacnet_transport::bbmd_lookup::BbmdLookupFailure;
use bacnet_transport::bip::{ForeignRegistrationOutcome, ForeignRegistrationSnapshot};
use bacnet_transport::bip6::Bip6ForeignStatus;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// `registration` as the dict `foreign_registration()` returns.
pub(crate) fn registration_dict(registration: ForeignRegistrationSnapshot) -> PyResult<Py<PyAny>> {
    let seconds = |d: Option<Duration>| d.map(|d| d.as_secs_f64());
    let outcome = registration.last_outcome.map(|outcome| match outcome {
        ForeignRegistrationOutcome::Pending => "pending",
        ForeignRegistrationOutcome::Accepted => "accepted",
        ForeignRegistrationOutcome::Refused => "refused",
        ForeignRegistrationOutcome::TimedOut => "timed_out",
        ForeignRegistrationOutcome::LocalError => "local_error",
        ForeignRegistrationOutcome::Cancelled => "cancelled",
    });
    crate::py_async::attach(|py| {
        let dict = PyDict::new(py);
        dict.set_item("registered", registration.registered)?;
        dict.set_item("lease_remaining", seconds(registration.lease_remaining))?;
        dict.set_item("next_attempt_in", seconds(registration.next_attempt_in))?;
        dict.set_item("last_bbmd", registration.last_bbmd.map(|a| a.to_string()))?;
        dict.set_item("last_result", registration.last_result.map(|c| c.to_raw()))?;
        dict.set_item("last_outcome", outcome)?;
        dict.set_item("attempts", registration.attempts)?;
        dict.set_item("accepted", registration.accepted)?;
        dict.set_item("refused", registration.refused)?;
        dict.set_item("lapses", registration.lapses)?;
        dict.set_item("renewal_hints", registration.renewal_hints)?;
        dict.set_item("bbmd_lookups", registration.bbmd_lookups)?;
        dict.set_item("bbmd_lookup_failures", registration.bbmd_lookup_failures)?;
        dict.set_item(
            "last_bbmd_lookup_failure",
            registration
                .last_bbmd_lookup_failure
                .map(|failure| lookup_failure(failure, Family::Ipv4)),
        )?;
        dict.set_item("waiting_for_bbmd", registration.waiting_for_bbmd)?;
        Ok(dict.into_any().unbind())
    })
}

/// `status` as the dict a B/IPv6 owner's `foreign_registration()`
/// returns (#2365). B/IPv6 doesn't track the BBMD's answers, so it reports
/// the BBMD's address in use and the host name it is known by, the wait for
/// that name, the lookups, the
/// releases of BBMDs left and the broadcasts refused while waiting.
pub(crate) fn bip6_registration_dict(status: Bip6ForeignStatus) -> PyResult<Py<PyAny>> {
    crate::py_async::attach(|py| {
        let dict = PyDict::new(py);
        dict.set_item("bbmd", status.bbmd.map(|bbmd| bbmd.to_string()))?;
        dict.set_item("bbmd_name", status.bbmd_name.map(|name| name.to_string()))?;
        dict.set_item("waiting_for_bbmd", status.waiting_for_bbmd)?;
        dict.set_item("bbmd_lookups", status.lookups.lookups)?;
        dict.set_item("bbmd_lookup_failures", status.lookups.failures)?;
        dict.set_item(
            "last_bbmd_lookup_failure",
            status
                .lookups
                .last_failure
                .map(|failure| lookup_failure(failure, Family::Ipv6)),
        )?;
        dict.set_item("previous_releases", status.releases.releases)?;
        dict.set_item("previous_released", status.releases.released)?;
        dict.set_item("previous_release_refused", status.releases.refused)?;
        dict.set_item("previous_release_unanswered", status.releases.unanswered)?;
        dict.set_item("broadcasts_refused", status.broadcasts_refused)?;
        dict.set_item("registrations_sent", status.registrations_sent)?;
        Ok(dict.into_any().unbind())
    })
}

/// The transport whose lookup failed: B/IP keeps IPv4 answers, B/IPv6
/// IPv6 ones.
#[derive(Clone, Copy)]
enum Family {
    Ipv4,
    Ipv6,
}

/// Why a lookup of the BBMD's host name failed (#2308), as the
/// `last_bbmd_lookup_failure` value names it. The resolver's own error
/// keeps only its kind, which for the system resolver says nothing more
/// than that the name did not resolve.
fn lookup_failure(failure: BbmdLookupFailure, family: Family) -> &'static str {
    match (failure, family) {
        (BbmdLookupFailure::Error(_), _) => "not_resolved",
        (BbmdLookupFailure::TimedOut, _) => "timed_out",
        (BbmdLookupFailure::NoAddress, Family::Ipv4) => "no_ipv4_address",
        (BbmdLookupFailure::NoAddress, Family::Ipv6) => "no_ipv6_address",
        (BbmdLookupFailure::Unusable(_), _) => "unusable_address",
        (BbmdLookupFailure::Abandoned, _) => "abandoned",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io;
    use std::net::Ipv4Addr;

    use bacnet_transport::bbmd_lookup::BbmdLookupFailure;

    use super::{Family, lookup_failure};

    /// The words `last_bbmd_lookup_failure` takes in the stub's TypedDict
    /// `class_name`.
    fn listed(class_name: &str) -> BTreeSet<&'static str> {
        let stub = include_str!("../rusty_bacnet.pyi");
        let class = &stub[stub
            .find(&format!("class {class_name}("))
            .expect("the class")..];
        let field = "last_bbmd_lookup_failure: Optional[Literal[";
        let rest = &class[class.find(field).expect("the field")..];
        let literal = &rest[..rest.find("]]").expect("the Literal's end")];
        literal.split('"').skip(1).step_by(2).collect()
    }

    /// The words `last_bbmd_lookup_failure` takes are the closed sets the
    /// stub's TypedDicts list (#2308, #2365): one for each failure, and
    /// `"other"` for one this version doesn't name.
    #[test]
    fn lookup_failures_take_the_stubs_words() {
        for (class, family) in [
            ("ForeignRegistrationStatus", Family::Ipv4),
            ("Bip6ForeignRegistrationStatus", Family::Ipv6),
        ] {
            let mut named: BTreeSet<&str> = [
                BbmdLookupFailure::Error(io::ErrorKind::NotFound),
                BbmdLookupFailure::TimedOut,
                BbmdLookupFailure::NoAddress,
                BbmdLookupFailure::Unusable(Ipv4Addr::BROADCAST.into()),
                BbmdLookupFailure::Abandoned,
            ]
            .into_iter()
            .map(|failure| lookup_failure(failure, family))
            .collect();
            named.insert("other");
            assert_eq!(named, listed(class), "{class}");
        }
    }
}
