use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::MacAddr;

/// Logical inbound source, not an authenticated principal (including on SC).
/// Shared only by exact duplicate detection and top-level request admission.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum CanonicalRequester {
    Direct(MacAddr),
    Routed(NpduAddress),
}

pub(super) fn canonical_requester(
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
) -> CanonicalRequester {
    source_network
        .filter(|source| (1..=0xfffe).contains(&source.network) && !source.mac_address.is_empty())
        .cloned()
        .map(CanonicalRequester::Routed)
        .unwrap_or_else(|| CanonicalRequester::Direct(MacAddr::from_slice(source_mac)))
}
