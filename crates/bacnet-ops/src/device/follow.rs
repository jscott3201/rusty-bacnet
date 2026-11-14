//! Following an instance: the client's record of where each followed
//! instance is reached, and a following handle looking for its device
//! again after a request finds it unreachable.

use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

use super::resolve::ambiguous;
use super::{ClientEvent, Device, DeviceAddress, Pin};
use crate::client::{Client, lock, unreachable};

impl<T: TransportPort + 'static> Device<T> {
    /// Where to send a request once more after `result` at `at`: for a
    /// following handle whose request found the device unreachable, the
    /// one other address claiming its instance ([`Client::relocate`]);
    /// `None` to keep `result`. A pinned handle never moves.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more other addresses claim it.
    pub(crate) async fn retry<R>(
        &self,
        at: &DeviceAddress,
        result: &Result<R, Error>,
    ) -> Result<Option<DeviceAddress>, Error> {
        match (&self.pin, result) {
            (Pin::Follows(instance), Err(error)) if unreachable(error) => {
                self.client.relocate(*instance, at).await
            }
            _ => Ok(None),
        }
    }
}

impl<T: TransportPort + 'static> Client<T> {
    /// Record that `instance` is followed at `address`, telling the events
    /// receivers when it was followed elsewhere before.
    pub(super) fn follow(&self, instance: u32, address: DeviceAddress) {
        let before = lock(&self.shared.follows).insert(instance, address.clone());
        if let Some(from) = before.filter(|from| *from != address) {
            let _ = self.shared.events.send(ClientEvent::Moved {
                instance,
                from,
                to: address,
            });
        }
    }

    /// Where `instance` is now, for a following handle that found it
    /// unreachable at `from`: the one other address claiming it, followed
    /// from now on; `None` when no other does.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more other addresses claim it.
    pub(crate) async fn relocate(
        &self,
        instance: u32,
        from: &DeviceAddress,
    ) -> Result<Option<DeviceAddress>, Error> {
        let claims = self.claims(instance).await;
        let others: Vec<&DeviceAddress> = claims.iter().filter(|a| *a != from).collect();
        match others.as_slice() {
            [] => Ok(None),
            [to] => {
                let to = (*to).clone();
                self.follow(instance, to.clone());
                Ok(Some(to))
            }
            _ => Err(ambiguous(instance, &claims)),
        }
    }
}
