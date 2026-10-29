//! The hub's one receiver of COV notifications: each goes to the entry
//! whose subscriber process identifier it carries, if it comes from that
//! entry's device and names its object (and property). The loop's arms
//! never await: applying a notification takes two std mutexes in turn (the
//! table's, released before the entry's), and a held-back signal is a
//! deadline the loop sleeps toward.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bacnet_client::client::{CovTarget, ReceivedCOVNotification};
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetPropertyValue;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::watch;
use tokio::time::Instant;

use bacnet_types::enums::PropertyIdentifier;

use super::options::MAX_ROWS;
use super::shared::{Registry, Shared};
use super::{Point, Watched};
use crate::format::{RawRow, decode_all};

/// Route notifications until the hub stops or the client closes its
/// channel.
pub(super) async fn run(
    registry: Arc<Registry>,
    mut notifications: broadcast::Receiver<ReceivedCOVNotification>,
    mut stop: watch::Receiver<bool>,
) {
    let mut held: Vec<(Instant, Arc<Shared>)> = Vec::new();
    loop {
        let due = held.iter().map(|(at, _)| *at).min();
        tokio::select! {
            biased;
            _ = stop.wait_for(|stopped| *stopped) => break,
            received = notifications.recv() => match received {
                Ok(received) => held.extend(registry.deliver(&received)),
                Err(RecvError::Lagged(missed)) => {
                    registry.lagged.fetch_add(missed, Ordering::Relaxed);
                }
                Err(RecvError::Closed) => break,
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

/// True when `received` is for `point`: from its device's address (see
/// [`CovTarget`]), about its object, and for a property watch, carrying
/// that property.
pub(super) fn belongs(
    point: &Point,
    target: Option<&CovTarget>,
    received: &ReceivedCOVNotification,
) -> bool {
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
    let local_from = |mac: &MacAddr| {
        received.source_network.is_none()
            && received.source_address.is_none()
            && received.source_mac == *mac
    };
    match target {
        Some(CovTarget::Local { mac }) => local_from(mac),
        // A routed device on the client's own network is sent to as a
        // local one, and may answer from its address as the MAC.
        Some(CovTarget::Routed {
            network, address, ..
        }) => {
            (received.source_network == Some(*network)
                && received.source_address.as_ref() == Some(address))
                || local_from(address)
        }
        _ => false,
    }
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
/// [`Reads::property_flags`]: super::Reads::property_flags
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
