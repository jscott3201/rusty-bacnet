//! Sending WriteGroup requests (Clause 15.11).
//!
//! WriteGroup is unconfirmed, so it can go to one device or to many at once:
//! the local network, one remote network through its routers, or every
//! network. Each receiving device that has Channels in the request's control
//! group applies the change list to them itself; nothing comes back.
use super::*;
use bacnet_services::write_group::WriteGroupRequest;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send a WriteGroup request to `scope` (#2588): every device on this
    /// network, on one remote network, or on every network. The standard
    /// lets WriteGroup go by unicast, multicast or broadcast, and prefers
    /// broadcasts to one network at a time over a global one (Clause
    /// 15.11); this client sends the unicast ([`Self::write_group_directed`])
    /// and the broadcasts, not a multicast.
    ///
    /// The service is unconfirmed: this returns once the request is sent, and
    /// no device answers it, so delivery isn't confirmed either. Fails before
    /// anything is sent when `request` doesn't encode (see
    /// [`WriteGroupRequest::encode`]) or the scope is
    /// [`Broadcast::Network`] 0 or 65535, which name no one network.
    pub async fn write_group(
        &self,
        scope: Broadcast,
        request: &WriteGroupRequest,
    ) -> Result<(), Error> {
        self.tell_all(scope, request).await
    }

    /// Send a WriteGroup request to the station `to` alone, as
    /// [`Self::write_group`] does to a scope.
    pub async fn write_group_directed(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &WriteGroupRequest,
    ) -> Result<(), Error> {
        self.tell(to, request).await
    }
}

#[cfg(test)]
#[path = "write_group_tests.rs"]
mod tests;
