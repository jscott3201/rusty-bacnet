//! The read services against a borrowed [`ObjectDatabase`]: what a request
//! may expand, how a host serves an object differently from the object
//! itself, and the staged steps (plan, sample, read) the host runs between
//! its awaits.
//!
//! [`ObjectDatabase`]: bacnet_objects::database::ObjectDatabase

use bacnet_objects::traits::BACnetObject;
use bacnet_types::primitives::ObjectIdentifier;

mod budget;
pub mod group;
mod property;
mod rpm;

pub use budget::{ReadFailure, ReadPropertyMultipleBudget};
pub use property::{
    PropertyPlan, plan_property, read_property_ack, read_target_object, read_value, resolve_target,
    rpm_response_index,
};
pub use rpm::RpmPlan;

/// Plain per-request read settings, apart from how objects are served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadScope {
    /// Result rows a single-property read may expand: its own row plus, for
    /// a Group's whole Present_Value, every member row (#1172). A read past
    /// it fails in planning, before any value is read.
    pub work_limit: usize,
    /// The port the Network_Port wildcard instance names, when one is
    /// registered. The wildcard resolves only to it.
    pub registered_port: Option<ObjectIdentifier>,
    /// The executor's fixed Device, which the Device wildcard instance names
    /// even when its object is gone. `None` falls back to the database's
    /// selected Device.
    pub device: Option<ObjectIdentifier>,
}

impl ReadScope {
    /// No work limit, no registered port, and the database's selected
    /// Device for the wildcard: a read with no executor around it.
    pub const UNLIMITED: Self = Self {
        work_limit: usize::MAX,
        registered_port: None,
        device: None,
    };
}

/// How a host serves a stored object differently from the object itself.
///
/// The server serves the executor-owned Device properties (its services,
/// Property_List, the live COV lists and address bindings, the frozen
/// identity) from a view over the stored Device. The read functions are
/// generic over the overlay, so a view per result row costs no allocation.
pub trait ReadOverlay {
    /// The served form of a stored object.
    type View<'a>: BACnetObject
    where
        Self: 'a;

    /// The served form of `stored`; `None` serves `stored` as it is.
    fn view<'a>(&'a self, stored: &'a dyn BACnetObject) -> Option<Self::View<'a>>;
}

/// No overlay: every object answers for itself. Pass `None::<&NoOverlay>`
/// where a read takes an optional overlay.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoOverlay;

impl ReadOverlay for NoOverlay {
    type View<'a> = unserved::Unserved;

    fn view<'a>(&'a self, _: &'a dyn BACnetObject) -> Option<unserved::Unserved> {
        None
    }
}

/// [`NoOverlay`]'s view, reachable only as `<NoOverlay as ReadOverlay>::View`.
mod unserved {
    use bacnet_objects::traits::BACnetObject;
    use bacnet_types::enums::PropertyIdentifier;
    use bacnet_types::error::Error;
    use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

    /// A view that is never made: [`NoOverlay`](super::NoOverlay) serves
    /// every object as it is.
    #[derive(Debug)]
    pub enum Unserved {}

    impl BACnetObject for Unserved {
        fn object_identifier(&self) -> ObjectIdentifier {
            match *self {}
        }
        fn object_name(&self) -> &str {
            match *self {}
        }
        fn read_property(
            &self,
            _: PropertyIdentifier,
            _: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            match *self {}
        }
        fn write_property(
            &mut self,
            _: PropertyIdentifier,
            _: Option<u32>,
            _: PropertyValue,
            _: Option<u8>,
        ) -> Result<(), Error> {
            match *self {}
        }
        fn property_list(&self) -> alloc::borrow::Cow<'static, [PropertyIdentifier]> {
            match *self {}
        }
    }
}

/// Which server-owned Device lists one request may select: the two COV lists
/// and Device_Address_Binding (#1369).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveLists {
    /// The selected local Device.
    pub device: ObjectIdentifier,
    /// `Active_COV_Subscriptions` may be read.
    pub active_cov: bool,
    /// `Active_COV_Multiple_Subscriptions` may be read.
    pub active_cov_multiple: bool,
    /// `Device_Address_Binding` may be read. It comes from the server's
    /// binding table, not the COV table.
    pub address_bindings: bool,
}

impl LiveLists {
    /// Whether either COV list is selected, so the COV table is sampled.
    pub fn reads_cov(self) -> bool {
        self.active_cov || self.active_cov_multiple
    }
}
