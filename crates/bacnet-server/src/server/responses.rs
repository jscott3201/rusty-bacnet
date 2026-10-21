use super::*;
use bacnet_network::layer::IssuedApdu;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Terminal ordinary response: release the pending owner only after NPDU
    /// encoding, at local issuance. Failure/cancellation also drops ownership.
    ///
    /// Returns the issuance future itself rather than awaiting it in an
    /// `async fn` of its own, which would add this function's arguments to
    /// every response's handler future (#1713).
    pub(super) fn issue_terminal_response<'a>(
        network: &'a NetworkLayer<T>,
        apdu: &'a [u8],
        source_mac: &'a [u8],
        source_network: Option<&'a NpduAddress>,
        route: &'a bacnet_network::response_route::ResponseRoute,
        pending: Option<PendingConfirmedRequest>,
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send + 'a {
        network.send_response_apdu_on_issuance(
            IssuedApdu {
                apdu,
                next_hop: source_mac,
                destination: source_network,
                expecting_reply: false,
                priority: NetworkPriority::Normal,
            },
            route,
            move || drop(pending),
        )
    }

    pub(super) async fn send_confirmed_response_apdu(
        network: &NetworkLayer<T>,
        apdu: &[u8],
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        route: &bacnet_network::response_route::ResponseRoute,
    ) -> Result<(), Error> {
        Self::send_confirmed_response_apdu_expecting_reply(
            network,
            apdu,
            source_mac,
            source_network,
            route,
            false,
        )
        .await
    }

    pub(super) async fn send_confirmed_response_apdu_expecting_reply(
        network: &NetworkLayer<T>,
        apdu: &[u8],
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        route: &bacnet_network::response_route::ResponseRoute,
        expecting_reply: bool,
    ) -> Result<(), Error> {
        network
            .send_response_apdu_on_issuance(
                IssuedApdu {
                    apdu,
                    next_hop: source_mac,
                    destination: source_network,
                    expecting_reply,
                    priority: NetworkPriority::Normal,
                },
                route,
                || {},
            )
            .await
    }
}
