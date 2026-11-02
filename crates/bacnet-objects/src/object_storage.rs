//! Concrete storage access without imposing 'static on borrowed read views.
/// Permit constructed only inside bacnet-objects. No external downcast.
#[doc(hidden)]
pub struct ObjectStorageAccess(pub(crate) ());

/// Blanket implementation seals concrete reflection against adapter overrides.
#[doc(hidden)]
pub trait StoredObject {
    /// Only owned database objects can use this crate-authorized reflection.
    fn as_stored_any(&self, access: ObjectStorageAccess) -> &dyn core::any::Any
    where
        Self: 'static;

    /// The mutable form of [`as_stored_any`](Self::as_stored_any).
    fn as_stored_any_mut(&mut self, access: ObjectStorageAccess) -> &mut dyn core::any::Any
    where
        Self: 'static;
}
impl<T> StoredObject for T {
    fn as_stored_any(&self, _: ObjectStorageAccess) -> &dyn core::any::Any
    where
        Self: 'static,
    {
        self
    }

    fn as_stored_any_mut(&mut self, _: ObjectStorageAccess) -> &mut dyn core::any::Any
    where
        Self: 'static,
    {
        self
    }
}
