use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) async fn send_confirmed_response_apdu(
        network: &NetworkLayer<T>,
        apdu: &[u8],
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
    ) -> Result<(), Error> {
        Self::send_confirmed_response_apdu_expecting_reply(
            network,
            apdu,
            source_mac,
            source_network,
            false,
        )
        .await
    }

    pub(super) async fn send_confirmed_response_apdu_expecting_reply(
        network: &NetworkLayer<T>,
        apdu: &[u8],
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        expecting_reply: bool,
    ) -> Result<(), Error> {
        if let Some(destination) = source_network {
            network
                .send_apdu_routed(
                    apdu,
                    destination.network,
                    &destination.mac_address,
                    source_mac,
                    expecting_reply,
                    NetworkPriority::NORMAL,
                )
                .await
        } else {
            network
                .send_apdu(apdu, source_mac, expecting_reply, NetworkPriority::NORMAL)
                .await
        }
    }
}
