//! Original-ingress authority for an application response.
use crate::layer::ReceivedApdu;
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_npdu::NetworkPriority;
use bacnet_transport::port::{DirectResponse, TransportProvenance};
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};

#[cfg(test)]
#[path = "response_encode_tests.rs"]
mod encode_tests;

#[cfg(test)]
#[path = "response_budget_tests.rs"]
mod budget_tests;

#[cfg(test)]
mod size_tests {
    use super::*;
    use bacnet_transport::port::ReceivedNpdu;

    /// Every received APDU is moved through two channels and into a
    /// handler, and its response route is saved with each confirmed
    /// request. The direct and router capabilities are one pointer each
    /// (#1713); inline they were 88 bytes apiece. Shrink a type that grows
    /// past its limit rather than raise the limit.
    #[test]
    fn received_envelopes_and_routes_stay_small() {
        let sizes = [
            ("ReceivedApdu", std::mem::size_of::<ReceivedApdu>(), 208),
            ("ReceivedNpdu", std::mem::size_of::<ReceivedNpdu>(), 160),
            ("ResponseRoute", std::mem::size_of::<ResponseRoute>(), 64),
        ];
        for (name, size, limit) in sizes {
            assert!(size <= limit, "{name} is {size} bytes, over {limit}");
        }
    }
}

/// Saved transport authority and addressing for a response.
///
/// Verified direct ingress requires a matching original-socket capability.
/// Missing or mismatched capability fails closed. Router delivery also retains
/// its addressed endpoint and original return port/peer. Clones own neither the
/// router nor a socket and do not extend either lifetime.
#[derive(Clone, Debug)]
pub struct ResponseRoute {
    provenance: TransportProvenance,
    direct: Option<DirectResponse>,
    router: Option<crate::router::RouterResponse>,
}

impl ResponseRoute {
    /// Capture immutable ingress provenance and its optional direct capability.
    /// A supplied capability is checked against provenance at every send.
    pub fn new(provenance: TransportProvenance, direct: Option<DirectResponse>) -> Self {
        Self {
            provenance,
            direct,
            router: None,
        }
    }

    /// Plain, non-direct addressing for locally constructed legacy envelopes.
    pub fn unverified() -> Self {
        Self::new(TransportProvenance::unverified(), None)
    }

    /// Original authentication scope; this never follows a replacement peer.
    pub fn provenance(&self) -> TransportProvenance {
        self.provenance.clone()
    }

    /// Cap an APDU by the captured router port and direct peer NPDU/BVLC limits,
    /// including both source and destination headers actually used by a reply.
    /// Ordinary ingress keeps the supplied APDU cap. Invalid authority/addressing
    /// is an error; retirement does not invalidate this immutable sizing snapshot.
    pub fn max_apdu_length(
        &self,
        apdu_limit: u16,
        destination: Option<&NpduAddress>,
    ) -> Result<u16, Error> {
        let direct = self.direct()?;
        let header = self.header_len(destination)?;
        let mut npdu_limit = usize::MAX;
        if let Some(router) = &self.router {
            npdu_limit = usize::from(router.max_npdu());
        }
        if let Some(direct) = direct {
            npdu_limit = npdu_limit.min(direct.max_npdu_length());
        }
        Ok(usize::from(apdu_limit).min(npdu_limit.saturating_sub(header)) as u16)
    }

    /// The NPCI octets [`Self::encode`] puts before a reply's APDU, found
    /// without allocating (#1713). A header the encoder refuses, or one past
    /// the scratch buffer, takes the owned path for its error or length.
    fn header_len(&self, destination: Option<&NpduAddress>) -> Result<usize, Error> {
        let mut scratch = [0u8; 64];
        let written = match &self.router {
            Some(router) => router
                .plan()
                .npdu(&[], false, NetworkPriority::Normal)
                .encode_into(&mut scratch),
            None => {
                check_response_destination(destination)?;
                response_npdu(&[], destination, false, NetworkPriority::Normal)
                    .encode_into(&mut scratch)
            }
        };
        match written {
            Ok(len) => Ok(len),
            Err(_) => self
                .encode(&[], destination, false, NetworkPriority::Normal)
                .map(|header| header.len()),
        }
    }

    /// Whether direct-socket authority must be checked, including a malformed
    /// context with a missing or mismatched capability.
    pub fn requires_direct_response(&self) -> bool {
        self.provenance.is_direct_peer() || self.direct.is_some()
    }

    /// Whether response issuance requires captured router or direct authority.
    pub fn requires_original_route(&self) -> bool {
        self.router.is_some() || self.provenance.is_direct_peer() || self.direct.is_some()
    }

    /// Compare the opening transport origin and router endpoint of a request.
    /// A later segment cannot switch the addressed endpoint or router instance.
    /// Direct capability presence is checked separately at each immediate send;
    /// reassembly retains the opening capability for its completed request.
    pub fn same_origin(&self, other: &Self) -> bool {
        self.provenance == other.provenance
            && match (&self.router, &other.router) {
                (Some(a), Some(b)) => a.same_origin(b),
                (None, None) => true,
                _ => false,
            }
    }

    /// Encode a response for an immediate MS/TP handoff. This validates all
    /// captured authority and router lifetime before returning any bytes.
    /// Direct SC responses must use network issuance, never the DER channel.
    pub fn encode_reply_npdu(
        &self,
        apdu: &[u8],
        destination: Option<&NpduAddress>,
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<BytesMut, Error> {
        if self.direct()?.is_some() {
            return Err(Error::Encoding(
                "a direct response requires its original socket".into(),
            ));
        }
        if let Some(router) = &self.router {
            router.check()?;
        }
        self.encode(apdu, destination, expecting_reply, priority)
    }

    pub(crate) fn router(&self) -> Option<&crate::router::RouterResponse> {
        self.router.as_ref()
    }

    /// Shared immutable plan, also used without liveness for sizing snapshots.
    pub(crate) fn encode(
        &self,
        apdu: &[u8],
        destination: Option<&NpduAddress>,
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<BytesMut, Error> {
        if let Some(router) = &self.router {
            let npdu = router.plan().npdu(apdu, expecting_reply, priority);
            let mut encoded = BytesMut::zeroed(npdu.encoded_len());
            npdu.encode_into(&mut encoded)
                .map_err(|error| Error::Encoding(error.to_string()))?;
            return Ok(encoded);
        }
        encode_response_npdu(apdu, destination, expecting_reply, priority)
    }

    pub(crate) fn direct(&self) -> Result<Option<&DirectResponse>, bacnet_types::error::Error> {
        match (self.provenance.direct_sc_identity(), self.direct.as_ref()) {
            (Some(identity), Some(route)) if identity == route.identity() => Ok(Some(route)),
            (None, None) => Ok(None),
            _ => Err(bacnet_types::error::Error::Transport(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "missing or mismatched original direct response capability",
            ))),
        }
    }
}

impl ReceivedApdu {
    /// Restore the complete opening response context after reassembly.
    pub fn set_response_route(&mut self, route: ResponseRoute) {
        self.provenance = route.provenance;
        self.direct_response = route.direct;
        self.router_response = route.router;
    }

    /// Save this ingress's response route before moving it into queued work.
    pub fn response_route(&self) -> ResponseRoute {
        ResponseRoute {
            provenance: self.provenance.clone(),
            direct: self.direct_response.clone(),
            router: self.router_response.clone(),
        }
    }
}

/// Shared framing for budget calculation and actual response issuance.
pub(crate) fn encode_response_npdu(
    apdu: &[u8],
    destination: Option<&NpduAddress>,
    expecting_reply: bool,
    priority: NetworkPriority,
) -> Result<BytesMut, Error> {
    check_response_destination(destination)?;
    encode_checked_response_npdu(apdu, destination, expecting_reply, priority)
}

/// Refuse a response `destination` on network 0 or 0xFFFF.
pub(crate) fn check_response_destination(destination: Option<&NpduAddress>) -> Result<(), Error> {
    match destination {
        Some(destination) => crate::layer::check_destination(
            destination.network,
            "pass no destination (no DNET) for a local peer",
        ),
        None => Ok(()),
    }
}

/// [`encode_response_npdu`] once [`check_response_destination`] has passed.
pub(crate) fn encode_checked_response_npdu(
    apdu: &[u8],
    destination: Option<&NpduAddress>,
    expecting_reply: bool,
    priority: NetworkPriority,
) -> Result<BytesMut, Error> {
    // Encoded straight from the borrowed APDU into a buffer of the NPDU's
    // size: no copy of the APDU first, no regrowth (#1713).
    let npdu = response_npdu(apdu, destination, expecting_reply, priority);
    let mut encoded = BytesMut::zeroed(npdu.encoded_len());
    if npdu.encode_into(&mut encoded).is_ok() {
        return Ok(encoded);
    }
    // A refused NPDU reports the owned encoder's error.
    let npdu = Npdu {
        destination: destination.cloned(),
        expecting_reply,
        priority,
        payload: Bytes::copy_from_slice(apdu),
        ..Npdu::default()
    };
    let mut encoded = BytesMut::new();
    encode_npdu(&mut encoded, &npdu)?;
    Ok(encoded)
}

/// The NPDU a reply carrying `apdu` to `destination` is.
fn response_npdu<'a>(
    apdu: &'a [u8],
    destination: Option<&'a NpduAddress>,
    expecting_reply: bool,
    priority: NetworkPriority,
) -> bacnet_npdu::Npdu<'a> {
    let mut npdu = bacnet_npdu::Npdu::apdu(apdu);
    npdu.destination = destination.map(|destination| bacnet_npdu::Address {
        network: destination.network,
        mac: &destination.mac_address,
    });
    npdu.expecting_reply = expecting_reply;
    npdu.priority = priority;
    npdu
}
