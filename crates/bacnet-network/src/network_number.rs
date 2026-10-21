//! Shared packet contract for the two local nonrouter Network Number controls.
use crate::layer::ReceivedNetworkControl;
use bacnet_npdu::NetworkMessageType;
use bacnet_router_core::message::NetworkNumberIs;
use bacnet_types::network_number::NetworkNumber;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

/// A validated local control, independent of transport and configured authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberControl {
    /// A local query received by unicast or logical broadcast.
    WhatIs,
    /// A valid logical-broadcast announcement.
    NumberIs {
        /// Peer-reported number, in 1..=65534.
        number: u16,
        /// Peer-reported configured flag, zero or one.
        flag: u8,
    },
}
impl NumberControl {
    /// Refuse routed, malformed or ineligible controls without changing state.
    pub fn parse(control: &ReceivedNetworkControl) -> Option<Self> {
        let npdu = &control.npdu;
        if !npdu.is_network_message() || npdu.source.is_some() || npdu.destination.is_some() {
            return None;
        }
        match npdu.message_type {
            Some(t)
                if t == NetworkMessageType::WHAT_IS_NETWORK_NUMBER && npdu.payload.is_empty() =>
            {
                Some(Self::WhatIs)
            }
            Some(t) if t == NetworkMessageType::NETWORK_NUMBER_IS && control.link_layer_group => {
                // The router core's view, the one reader of this payload
                // (#1818): it refuses any length but three octets.
                let NetworkNumberIs { network, flag } =
                    NetworkNumberIs::parse(&npdu.payload).ok()?;
                (network != 0 && network != u16::MAX && flag <= NetworkNumberIs::CONFIGURED)
                    .then_some(Self::NumberIs {
                        number: network,
                        flag,
                    })
            }
            _ => None,
        }
    }
}

/// Encode a known number as a complete local-broadcast control NPDU.
/// Learned-configured state still transmits flag zero; unknown has no reply.
pub fn number_is_reply(state: NetworkNumber) -> Option<[u8; 6]> {
    let (number, quality) = state.snapshot();
    if number == 0 {
        return None;
    }
    let [high, low, flag] = NetworkNumberIs {
        network: number,
        flag: u8::from(quality == 3),
    }
    .to_bytes();
    Some([
        1,
        0x80,
        NetworkMessageType::NETWORK_NUMBER_IS.to_raw(),
        high,
        low,
        flag,
    ])
}

/// The number of the network a single-port node is attached to, shared
/// between the task that owns its Number controls and the senders that read
/// it. Each [`NetworkLayer`](crate::layer::NetworkLayer) holds one
/// ([`NetworkLayer::local_network_number`](crate::layer::NetworkLayer::local_network_number)),
/// and clones share one value. Reading or publishing takes no lock.
///
/// It starts unknown. The layer never learns a number by itself: the owner of
/// its Number controls publishes the state it holds whenever an announcement
/// may have changed it, and a registered Network Port's owner also publishes
/// the port's number at startup. The value is a copy of that state, never a
/// second authority.
#[derive(Clone, Debug, Default)]
pub struct LocalNetworkNumber(Arc<AtomicU16>);

impl LocalNetworkNumber {
    /// Record the number `state` holds. An unknown state records unknown;
    /// a known number is always in 1..=65534.
    pub fn publish(&self, state: NetworkNumber) {
        self.0.store(state.snapshot().0, Ordering::Release);
    }

    /// The last number published, or `None` while it is unknown.
    pub fn get(&self) -> Option<u16> {
        match self.0.load(Ordering::Acquire) {
            0 => None,
            number => Some(number),
        }
    }
}

#[cfg(test)]
mod tests;
