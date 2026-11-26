//! What a device's scheduler shares with the hub's table and its
//! notification receiver: its inbox of points, a handle's move cue, and
//! what the receiver noted at the device's address since the scheduler last
//! looked (#2603). Each lock here is a std mutex held for a few field
//! updates, after the table's and before an entry's ([`shared`]).
//!
//! [`shared`]: super::shared

use std::sync::{Mutex, MutexGuard};

use bacnet_types::sync::MutexExt;
use tokio::sync::Notify;
use tokio::time::Instant;

use super::shared::Arrival;
use crate::device::DeviceAddress;

/// Announcements, and senders, a scheduler keeps between two looks, each
/// once in a row: more mark announcements missed (the scheduler reads the
/// table instead), or drop the oldest sender (a later notification names
/// it again).
const HEARD_KEPT: usize = 8;

/// What the notification receiver noted at a device's address since its
/// scheduler last looked (#2603).
#[derive(Debug, Default)]
pub(super) struct Notes {
    /// Instances announced there, in order, each with whether its I-Am is
    /// newer than any seen there before: only a new one may be a restart,
    /// not a row the table moved when the client learned its network.
    pub(super) announced: Vec<(u32, bool)>,
    /// Announcements were missed: the receiver fell behind, or more came
    /// between two looks than are kept.
    pub(super) missed: bool,
    /// Instances COV notifications from the address named as their sender
    /// although the device had named another for their point.
    pub(super) senders: Vec<u32>,
    /// A device at the address said it restarted (a restart notification):
    /// its instance. Only a cue to read its restart time.
    pub(super) restart: Option<u32>,
    /// Instances given up here for another device at the address, heard
    /// announcing themselves at another address since.
    pub(super) elsewhere: Vec<u32>,
}

/// A sender name the device at an address uses for itself in its
/// notifications (a gateway naming itself, say), taken for its points
/// after a read showed no device of that instance answers there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Alias {
    /// The device it stands for: only its points take the name.
    pub(super) device: u32,
    /// The instance the notifications name.
    pub(super) sender: u32,
    /// When it is checked again: taken until then.
    pub(super) until: Instant,
}

/// What the receiver keeps of a device's address.
#[derive(Debug)]
struct Heard {
    notes: Notes,
    /// The newest I-Am seen at the address, on the discovery table's
    /// clock: the slot's making at first.
    seen: std::time::Instant,
    alias: Option<Alias>,
}

/// Push `value` onto `list` unless it ends it already, keeping at most
/// [`HEARD_KEPT`]: whether it was pushed, and whether there was room.
fn keep<V: PartialEq>(list: &mut Vec<V>, value: V) -> (bool, bool) {
    if list.last() == Some(&value) {
        return (false, true);
    }
    let room = list.len() < HEARD_KEPT;
    if !room {
        list.remove(0);
    }
    list.push(value);
    (true, room)
}

/// A device's scheduler, as the table knows it: entries not yet taken,
/// where a handle following its instance moved it to, what was announced
/// at its address, and its wake-up.
pub(super) struct DeviceSlot {
    pub(super) inbox: Mutex<Vec<Arrival>>,
    /// Set by the notification receiver when a handle following an
    /// instance moved it from the device's address
    /// ([`ClientEvent::Moved`]): the instance, and where it went.
    ///
    /// [`ClientEvent::Moved`]: crate::device::ClientEvent::Moved
    pub(super) moved: Mutex<Option<(u32, DeviceAddress)>>,
    /// What the notification receiver noted at the address.
    heard: Mutex<Heard>,
    pub(super) wake: Notify,
}

impl DeviceSlot {
    pub(super) fn new() -> Self {
        Self {
            inbox: Mutex::new(Vec::new()),
            moved: Mutex::new(None),
            heard: Mutex::new(Heard {
                notes: Notes::default(),
                seen: tokio::time::Instant::now().into_std(),
                alias: None,
            }),
            wake: Notify::new(),
        }
    }

    pub(super) fn inbox(&self) -> MutexGuard<'_, Vec<Arrival>> {
        self.inbox.locked()
    }

    fn heard(&self) -> MutexGuard<'_, Heard> {
        self.heard.locked()
    }

    /// A device at the address announced itself as `instance`, by an I-Am
    /// the table took at `at`: whether that is news for the scheduler (a
    /// flood of the same announcement wakes it once).
    pub(super) fn hear(&self, instance: u32, at: Option<std::time::Instant>) -> bool {
        let mut heard = self.heard();
        let new = at.is_some_and(|at| at > heard.seen);
        if let Some(at) = at.filter(|_| new) {
            heard.seen = at;
        }
        let announced = &mut heard.notes.announced;
        match announced.last_mut() {
            Some((last, fresh)) if *last == instance => {
                let news = new && !*fresh;
                *fresh |= new;
                news
            }
            _ => {
                if !keep(announced, (instance, new)).1 {
                    heard.notes.missed = true;
                }
                true
            }
        }
    }

    /// Announcements at the address may have been missed.
    pub(super) fn miss(&self) {
        self.heard().notes.missed = true;
    }

    /// A notification from the address named `sender` as its device,
    /// another than its point's: whether that is news for the scheduler.
    pub(super) fn sender(&self, sender: u32) -> bool {
        keep(&mut self.heard().notes.senders, sender).0
    }

    /// A device at the address, `instance`, said it restarted: whether
    /// that is news for the scheduler.
    pub(super) fn restarted(&self, instance: u32) -> bool {
        self.heard().notes.restart.replace(instance) != Some(instance)
    }

    /// `instance`, given up here, announced itself elsewhere.
    pub(super) fn elsewhere(&self, instance: u32) {
        keep(&mut self.heard().notes.elsewhere, instance);
    }

    /// What was noted since the last look.
    pub(super) fn take_notes(&self) -> Notes {
        std::mem::take(&mut self.heard().notes)
    }

    /// Whether `sender` is a name the device at the address uses for
    /// `device` (see [`Alias`]), now.
    pub(super) fn takes(&self, device: u32, sender: u32) -> bool {
        self.heard().alias.is_some_and(|alias| {
            alias.device == device && alias.sender == sender && Instant::now() < alias.until
        })
    }

    /// The name the address's notifications use in place of their
    /// device's, if one is taken; `None` to take none.
    pub(super) fn set_alias(&self, alias: Option<Alias>) {
        self.heard().alias = alias;
    }

    /// The name taken, if any.
    pub(super) fn alias(&self) -> Option<Alias> {
        self.heard().alias
    }

    /// The instance a following handle moved from the device's address,
    /// and where to, once.
    pub(super) fn take_moved(&self) -> Option<(u32, DeviceAddress)> {
        self.moved.locked().take()
    }
}
