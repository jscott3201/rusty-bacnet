//! The hub's one receiver of COV notifications: each goes to the entry
//! whose subscriber process identifier it carries, if it comes from that
//! entry's device and names its object (and property). The hub subscribes
//! per object or property (SubscribeCOV, SubscribeCOVProperty), never with
//! SubscribeCOVPropertyMultiple, so a COV-multiple notification answers none
//! of its watches: it is left alone and counted
//! ([`HubStats::cov_multiple_ignored`](super::hub::HubStats::cov_multiple_ignored)).
//! The loop's arms
//! never await: applying a notification takes two std mutexes in turn (the
//! table's, released before the entry's), and a held-back signal is a
//! deadline the loop sleeps toward. It also hands each device's scheduler
//! the moves of handles following the device's instance away from its
//! address ([`ClientEvent::Moved`]).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bacnet_client::client::{COVEvent, ReceivedCOVNotification, Station};
use bacnet_types::constructed::BACnetPropertyValue;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::watch;
use tokio::time::Instant;

use bacnet_types::enums::PropertyIdentifier;

use super::options::MAX_ROWS;
use super::shared::{Registry, Shared};
use super::{Point, Watched};
use bacnet_client::client::{DeviceEvent, DeviceEventKind};

use crate::device::{ClientEvent, DeviceAddress};
use crate::format::{RawRow, decode_all};

/// What the receiver hears besides the notifications: the client's own
/// events and the discovery table's.
pub(super) struct Heard {
    /// Handles moving ([`ClientEvent::Moved`]).
    pub(super) events: broadcast::Receiver<ClientEvent>,
    /// Devices announcing themselves.
    pub(super) devices: broadcast::Receiver<DeviceEvent>,
    /// The client's own network's number, once known: an address on it is
    /// the local station, as the hub keys it.
    pub(super) own: Box<dyn Fn() -> Option<u16> + Send>,
}

/// Route notifications until the hub stops or the client closes its
/// channel.
pub(super) async fn run(
    registry: Arc<Registry>,
    mut notifications: broadcast::Receiver<COVEvent>,
    heard: Heard,
    mut stop: watch::Receiver<bool>,
) {
    let Heard {
        mut events,
        mut devices,
        own,
    } = heard;
    let mut held: Vec<(Instant, Arc<Shared>)> = Vec::new();
    let (mut following, mut announcing) = (true, true);
    loop {
        let due = held.iter().map(|(at, _)| *at).min();
        tokio::select! {
            biased;
            _ = stop.wait_for(|stopped| *stopped) => break,
            received = notifications.recv() => match received {
                Ok(event @ COVEvent::Object(_)) => held.extend(registry.deliver(&event, own())),
                Ok(COVEvent::Multiple(_)) => {
                    registry.cov_multiple_ignored.fetch_add(1, Ordering::Relaxed);
                }
                Err(RecvError::Lagged(missed)) => {
                    registry.lagged.fetch_add(missed, Ordering::Relaxed);
                }
                Err(RecvError::Closed) => break,
            },
            event = events.recv(), if following => match event {
                Ok(ClientEvent::Moved { instance, from, to }) => {
                    registry.moved(instance, &from, to);
                }
                // Another kind of event says nothing a watch needs.
                #[allow(unreachable_patterns)]
                Ok(_) => {}
                // A move missed is found by the device's next unanswered
                // request instead.
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => following = false,
            },
            change = devices.recv(), if announcing => match change {
                Ok(change) if change.kind != DeviceEventKind::Lost => {
                    if let Some(at) = DeviceAddress::of(&change.device) {
                        let instance = change.device.object_identifier.instance_number();
                        registry.announced(&at.localized(own()), instance);
                    }
                }
                // A loss, or announcements missed: a point given up waits
                // for the next.
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => announcing = false,
            },
            () = tokio::time::sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => {
                let now = Instant::now();
                held.retain(|(at, shared)| {
                    if *at <= now {
                        shared.flush();
                        false
                    } else {
                        true
                    }
                });
            }
        }
    }
}

/// True when `event` is for `point`: a single-object notification from its
/// device's address, by the client's rule ([`Station::is_source_of`]: its
/// network and MAC, whatever router it came through, or its MAC alone once
/// `own`, the client's network number, says the device is on this network),
/// about its object, and for a property watch, carrying that property.
pub(super) fn belongs(
    point: &Point,
    target: Option<&Station>,
    event: &COVEvent,
    own: Option<u16>,
) -> bool {
    let Some(received) = event.object() else {
        return false;
    };
    let notification = &received.notification;
    if notification.monitored_object_identifier != point.object {
        return false;
    }
    if matches!(point.watched, Watched::Property { .. })
        && !notification
            .list_of_values
            .iter()
            .any(|value| watched_value(point, value))
    {
        return false;
    }
    target.is_some_and(|target| target.is_source_of(event, own))
}

/// Whether `value` is one a point keeps: for an object watch, any value
/// (what is kept is limited in [`merge`]); for a property watch, its
/// property, at its element when it watches one. A value that names no
/// element is taken as that element only when it decodes to a single
/// value, as some devices send it; a whole array without an index is
/// dropped, keeping the previous value.
fn watched_value(point: &Point, value: &BACnetPropertyValue) -> bool {
    match point.watched {
        Watched::Object => true,
        Watched::Property { property, index } => {
            value.property_identifier == property
                && match index {
                    None => value.property_array_index.is_none(),
                    Some(index) => match value.property_array_index {
                        Some(i) => i == index,
                        None => decode_all(&value.value).is_some_and(|v| v.len() == 1),
                    },
                }
        }
    }
}

/// Whether `value` is the Status_Flags a property watch keeps beside its
/// property when the hub reads them ([`Reads::property_flags`]).
///
/// [`Reads::property_flags`]: super::hub::Reads::property_flags
fn flags_value(point: &Point, value: &BACnetPropertyValue, property_flags: bool) -> bool {
    property_flags
        && matches!(point.watched, Watched::Property { property, .. } if property != PropertyIdentifier::STATUS_FLAGS)
        && value.property_identifier == PropertyIdentifier::STATUS_FLAGS
        && value.property_array_index.is_none()
}

/// Fold a notification's values into `rows`: each reported value replaces
/// the row of the same property. An object watch takes in a new row only
/// for a property it reads (`object_reads`), up to [`MAX_ROWS`], so a
/// device sending many other properties can't grow it; a property watch
/// keeps its property, and its Status_Flags when `property_flags`. The
/// values folded in are returned.
pub(super) fn merge(
    rows: &mut Vec<RawRow>,
    point: &Point,
    object_reads: &[PropertyIdentifier],
    property_flags: bool,
    received: &ReceivedCOVNotification,
) -> Vec<RawRow> {
    let mut merged = Vec::new();
    for value in &received.notification.list_of_values {
        let flags = flags_value(point, value, property_flags);
        if !flags && !watched_value(point, value) {
            continue;
        }
        let property = value.property_identifier;
        let row = (property, Ok(value.value.clone()));
        if let Some(slot) = rows.iter_mut().find(|(p, _)| *p == property) {
            *slot = row.clone();
            merged.push(row);
            continue;
        }
        let joins = match point.watched {
            Watched::Object => object_reads.contains(&property) && rows.len() < MAX_ROWS,
            Watched::Property { .. } if flags => rows.len() < MAX_ROWS,
            // Only the watched property gets here, and it isn't a row yet.
            Watched::Property { .. } => rows.len() < MAX_ROWS,
        };
        if joins {
            rows.push(row.clone());
            merged.push(row);
        }
    }
    merged
}
