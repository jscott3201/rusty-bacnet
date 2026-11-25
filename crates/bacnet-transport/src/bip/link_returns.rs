//! A foreign device's link returns (#2572): whether a registration has
//! been lost, refused or unanswered, or a broadcast refused, since the
//! lease was last proven, and the count `TransportPort::link_returns`
//! reports. Kept in the transport's client record, beside the node, so the
//! node's own state, which every send and receive reads, keeps its layout.

use std::sync::Mutex;

use bacnet_bip_core::node::RegistrationEvent;
use bacnet_types::sync::MutexExt;
use tokio::sync::watch;

#[derive(Debug, Default)]
struct Follow {
    /// A registration was lost, refused or unanswered: broadcasts may have
    /// gone nowhere, so the next registration is the link coming back. Not
    /// set for the first attempt of a start, whose broadcasts are held.
    lost: bool,
    /// The BBMD refused a broadcast with a Distribute-Broadcast-To-Network
    /// NAK (a renewal hint): it holds no entry for this device, live lease
    /// or not, so the next success, even a renewal, is a return.
    dropped: bool,
}

/// The link's returns and what they follow.
#[derive(Debug)]
pub(super) struct LinkReturns {
    follow: Mutex<Follow>,
    count: watch::Sender<u64>,
}

impl Default for LinkReturns {
    fn default() -> Self {
        Self {
            follow: Mutex::default(),
            count: watch::Sender::new(0),
        }
    }
}

impl LinkReturns {
    /// Follow the registration through `event`. The first attempt's own
    /// success is no return: the broadcasts made before it were held and
    /// go now.
    pub(super) fn event(&self, event: &RegistrationEvent) {
        let mut follow = self.follow.locked();
        let back = match event {
            RegistrationEvent::Registered { .. } => {
                std::mem::take(&mut follow.lost) | std::mem::take(&mut follow.dropped)
            }
            RegistrationEvent::Renewed { .. } => {
                follow.lost = false;
                std::mem::take(&mut follow.dropped)
            }
            RegistrationEvent::Lapsed
            | RegistrationEvent::Refused { .. }
            | RegistrationEvent::Unanswered => {
                follow.lost = true;
                false
            }
            _ => false,
        };
        drop(follow);
        if back {
            self.count
                .send_modify(|count| *count = count.wrapping_add(1));
        }
    }

    /// The BBMD refused a broadcast: it holds no entry for this device.
    pub(super) fn broadcast_dropped(&self) {
        self.follow.locked().dropped = true;
    }

    /// A new start follows its own registration from scratch.
    pub(super) fn restart(&self) {
        *self.follow.locked() = Follow::default();
    }

    /// A receiver of the count.
    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.count.subscribe()
    }
}
