//! The watch list's messages: what the UI asks the watch worker for and
//! what the worker reports back, as plain data with no transport type.

use std::time::Instant;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use super::point::Health;

/// Identifies one watch row for the life of a session. The worker also uses
/// it as the row's subscriber process identifier.
pub(crate) type RowId = u32;

/// One watched property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Point {
    /// Device instance, from the discovery table.
    pub(crate) device: u32,
    /// The remote network the device is on, from the device table (or as
    /// typed, for a device not in it yet); `None` on this network.
    pub(crate) network: Option<u16>,
    /// The object.
    pub(crate) object: ObjectIdentifier,
    /// The property; Present_Value takes SubscribeCOV, any other property
    /// SubscribeCOVProperty.
    pub(crate) property: PropertyIdentifier,
}

impl Point {
    /// The device as the CLI's target syntax writes it: `2001`, or
    /// `2:2001` for one on network 2.
    pub(crate) fn device_label(&self) -> String {
        match self.network {
            Some(network) => format!("{network}:{}", self.device),
            None => self.device.to_string(),
        }
    }

    /// True when a SubscribeCOV on the whole object covers this property.
    /// The standard objects report Present_Value and Status_Flags to such a
    /// subscription (Table 13-1); anything else needs SubscribeCOVProperty.
    pub(crate) fn whole_object(&self) -> bool {
        self.property == PropertyIdentifier::PRESENT_VALUE
    }
}

/// UI to the watch worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WatchCommand {
    /// Start watching `point` as row `row`.
    Add {
        /// The new row.
        row: RowId,
        /// What it watches.
        point: Point,
    },
    /// Stop watching row `row` and cancel its subscription, if it has one.
    Remove {
        /// The row.
        row: RowId,
    },
    /// Start or stop logging samples to a CSV file in the profile workspace.
    Csv(bool),
}

/// Where a row's values come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// Reading the point and subscribing.
    Starting,
    /// COV notifications, with this finite lifetime in seconds, renewed
    /// before it runs out.
    Cov {
        /// Lifetime requested.
        lifetime: u32,
    },
    /// Polled, and why COV isn't used.
    Polled(String),
    /// The point can't be watched.
    Failed(String),
}

/// One value of a row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sample {
    /// The value as display text, or why it could not be read.
    pub(crate) value: Result<String, String>,
    /// The value as a number, for the sparkline.
    pub(crate) number: Option<f64>,
    /// Status_Flags, when the device sent them.
    pub(crate) flags: Option<Health>,
    /// When it arrived.
    pub(crate) at: Instant,
}

/// The watch worker's counters for the status bar and the watch screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WatchStats {
    /// COV subscriptions this session holds on devices, cancels in flight
    /// included: the remote footprint.
    pub(crate) subscriptions: usize,
    /// Confirmed requests sent in the last [`METER_WINDOW_SECS`] seconds,
    /// browser and watch list together.
    pub(crate) recent_requests: usize,
    /// The CSV file samples go to, when logging.
    pub(crate) csv: Option<String>,
}

/// The request-rate meter's window.
pub(crate) const METER_WINDOW_SECS: u64 = 10;

/// Watch worker to UI.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WatchEvent {
    /// A row's source changed.
    Source {
        /// The row.
        row: RowId,
        /// Its new source.
        source: Source,
    },
    /// What a row's first read found besides its value.
    Details {
        /// The row.
        row: RowId,
        /// The object's Object_Name, when the device gave it.
        name: Option<String>,
        /// The units as text, when the object has them.
        units: Option<String>,
    },
    /// New values, oldest first per row.
    Samples(Vec<(RowId, Sample)>),
    /// The counters changed.
    Stats(WatchStats),
    /// Something the user should see in the footer.
    Notice(String),
}
