use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) async fn send_confirmed_response_apdu(
        network: &NetworkLayer<T>,
        apdu: &[u8],
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
    ) -> Result<(), Error> {
        if let Some(destination) = source_network {
            network
                .send_apdu_routed(
                    apdu,
                    destination.network,
                    &destination.mac_address,
                    source_mac,
                    false,
                    NetworkPriority::NORMAL,
                )
                .await
        } else {
            network
                .send_apdu(apdu, source_mac, false, NetworkPriority::NORMAL)
                .await
        }
    }
}
