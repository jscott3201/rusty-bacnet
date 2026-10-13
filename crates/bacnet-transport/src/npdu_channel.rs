//! Handing a received NPDU up to the network layer, shared by the B/IP and
//! B/IPv6 receive paths (#1624, #1643).

use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc::{self, error::TrySendError};

use crate::port::ReceivedNpdu;

/// Why the network layer's channel refused an NPDU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The channel is full: the network layer is falling behind.
    Full,
    /// The network layer closed its end, as while the stack shuts down.
    Closed,
}

/// Hand an NPDU from `source_mac` up through `tx`, as a link-layer group
/// delivery when `link_layer_group`. The channel's slot is reserved before
/// `npdu` makes the NPDU's octets, so an NPDU the channel refuses costs no
/// copy during a flood.
pub(crate) fn hand_up(
    tx: &mpsc::Sender<ReceivedNpdu>,
    npdu: impl FnOnce() -> Bytes,
    source_mac: MacAddr,
    link_layer_group: bool,
) -> Result<(), Refused> {
    let permit = tx.try_reserve().map_err(|error| match error {
        TrySendError::Full(()) => Refused::Full,
        TrySendError::Closed(()) => Refused::Closed,
    })?;
    permit.send(ReceivedNpdu::unverified(
        npdu(),
        source_mac,
        link_layer_group,
        Vec::new(),
        None,
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];

    #[test]
    fn a_refused_npdu_is_never_copied() {
        let (tx, mut rx) = mpsc::channel(1);
        let mac = MacAddr::from_slice(&[1, 2, 3]);
        let sent = hand_up(&tx, || Bytes::from_static(NPDU), mac.clone(), true);
        assert_eq!(sent, Ok(()));
        let full = hand_up(&tx, || unreachable!("copied"), mac.clone(), false);
        assert_eq!(full, Err(Refused::Full));

        let received = rx.try_recv().unwrap();
        assert_eq!(received.npdu.as_ref(), NPDU);
        assert_eq!(received.source_mac, mac);
        assert!(received.link_layer_group);
        assert!(received.data_attributes.is_empty());
        assert!(received.reply_tx.is_none() && received.direct_response.is_none());

        rx.close();
        let closed = hand_up(&tx, || unreachable!("copied"), mac, false);
        assert_eq!(closed, Err(Refused::Closed));
    }
}
