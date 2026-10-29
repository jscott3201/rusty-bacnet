//! Sending WriteGroup requests (Clause 15.11).
//!
//! WriteGroup is unconfirmed, so it can go to one device or to many at once:
//! the local network, one remote network through its routers, or every
//! network. Each receiving device that has Channels in the request's control
//! group applies the change list to them itself; nothing comes back.
use super::*;
use bacnet_services::write_group::WriteGroupRequest;

/// Where [`BACnetClient::write_group`] sends a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteGroupDestination {
    /// One device, by its address on the local network.
    Device(MacAddr),
    /// Every device on the local network.
    LocalBroadcast,
    /// Every device on one remote network (1 to 65534), through its router.
    RemoteBroadcast(u16),
    /// Every device on every network (DNET 65535).
    GlobalBroadcast,
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send a WriteGroup request to `destination`.
    ///
    /// The service is unconfirmed: this returns once the request is sent, and
    /// no device answers it, so delivery isn't confirmed either. Fails before
    /// anything is sent when `request` doesn't encode (see
    /// [`WriteGroupRequest::encode`]) or a remote network number is 0 or
    /// 65535; use [`WriteGroupDestination::GlobalBroadcast`] for every
    /// network.
    pub async fn write_group(
        &self,
        destination: &WriteGroupDestination,
        request: &WriteGroupRequest,
    ) -> Result<(), Error> {
        if let WriteGroupDestination::RemoteBroadcast(network) = destination
            && !(1..u16::MAX).contains(network)
        {
            return Err(Error::Encoding(format!(
                "WriteGroup remote network {network} out of range 1-65534"
            )));
        }
        let mut service = BytesMut::new();
        request.encode(&mut service)?;
        let choice = UnconfirmedServiceChoice::WRITE_GROUP;
        match destination {
            WriteGroupDestination::Device(mac) => {
                self.unconfirmed_request(mac, choice, &service).await
            }
            WriteGroupDestination::LocalBroadcast => {
                self.broadcast_unconfirmed(choice, &service).await
            }
            WriteGroupDestination::RemoteBroadcast(network) => {
                self.broadcast_network_unconfirmed(choice, &service, *network)
                    .await
            }
            WriteGroupDestination::GlobalBroadcast => {
                self.broadcast_global_unconfirmed(choice, &service).await
            }
        }
    }
}

#[cfg(test)]
#[path = "write_group_tests.rs"]
mod tests;
