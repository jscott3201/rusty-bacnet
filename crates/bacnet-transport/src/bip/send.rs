//! What the transport sends for an NPDU: the node's link decides the
//! message and where it goes, without a lock; only a foreign device's
//! broadcast asks the node itself.

use bacnet_bip_codec as codec;
use bacnet_bip_core::node::Mode;
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::node::NodeHandle;
use super::own_broadcast::OwnBroadcastForwarder;
use crate::bvll::{bvll_error, encode_bvll};

impl NodeHandle {
    /// Send `npdu` to the B/IP address `mac` as an Original-Unicast-NPDU
    /// (Annex J.3). One of the link's group addresses is refused, unsent
    /// (#1677).
    pub(super) async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let dest = codec::address_from_slice(mac).map_err(bvll_error)?;
        let outgoing = self
            .link()
            .unicast(dest, npdu)
            .map_err(|refusal| self.refuse_send(refusal))?;

        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, &outgoing.message)?;

        self.socket()
            .send_to(&buf, outgoing.to)
            .await
            .map_err(Error::Transport)?;
        Ok(())
    }

    /// Broadcast `npdu` on this B/IP network: a foreign device sends it to
    /// its BBMD as Distribute-Broadcast-To-Network (Annex J.2.10); otherwise
    /// it goes out as an Original-Broadcast-NPDU to the subnet's broadcast
    /// address (Annex J.4.1), and a BBMD first hands it to `forwarder` for
    /// its BDT peers and foreign devices (Annex J.4.5).
    pub(super) async fn send_broadcast(
        &self,
        npdu: &[u8],
        forwarder: Option<&OwnBroadcastForwarder>,
    ) -> Result<(), Error> {
        let (outgoing, forward_own) = self
            .link()
            .broadcast(npdu)
            .map_err(|refusal| self.refuse_send(refusal))?;

        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, &outgoing.message)?;

        // A foreign device asks its node first, which holds the broadcast
        // while the first registration attempt after a start is outstanding,
        // refuses it when that hold is full (#1622), and queues it behind
        // released broadcasts its driver hasn't sent yet.
        if matches!(self.link().mode, Mode::Foreign(_)) && !self.admit_broadcast(npdu)? {
            return Ok(());
        }

        // The fanout is queued first, so its targets are fixed before the
        // local frame is on the wire. Whatever happens to it (see
        // OwnBroadcastForwarder::forward) never fails this send, and a
        // failed local send does not withdraw it.
        if forward_own {
            if let Some(forwarder) = forwarder {
                forwarder.forward(npdu);
            }
        }

        self.socket()
            .send_to(&buf, outgoing.to)
            .await
            .map_err(Error::Transport)?;
        Ok(())
    }
}
