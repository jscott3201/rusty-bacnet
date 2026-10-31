//! The application's own CreateObject constructors (#2173).

use super::*;
use bacnet_objects::traits::BACnetObject;
use std::collections::BTreeMap;

/// Builds a new object for CreateObject: given the identifier the request
/// names or the server picked, and a default name, it returns the object.
///
/// It runs synchronously with the object database write-locked, so it must
/// be quick and must not block, await or take a lock the database holder
/// could wait on. Build the object and return it; leave anything slow to
/// another task. See [`ObjectConstructors::register`] for how its answer is
/// used.
pub type ObjectConstructor =
    Arc<dyn Fn(ObjectIdentifier, &str) -> Result<Box<dyn BACnetObject>, Error> + Send + Sync>;

/// The object types an application makes creatable over the network, each
/// with its constructor, set in [`ServerConfig::object_constructors`] or
/// through a server builder's `object_constructor`.
///
/// CreateObject looks here before the built-in table,
/// [`bacnet_objects::creation`], so a registration may stand in for a
/// built-in type's constructor, for example to give new Analog Values
/// site-specific defaults. The registry is part of the configuration the
/// server starts with and doesn't change while it runs.
#[derive(Clone, Default)]
pub struct ObjectConstructors {
    by_type: BTreeMap<u32, ObjectConstructor>,
}

impl ObjectConstructors {
    /// An empty registry: CreateObject builds only the built-in types.
    pub fn new() -> Self {
        Self::default()
    }

    /// Make `object_type` creatable through `constructor`.
    ///
    /// Refused, leaving the registry unchanged, when the type already has a
    /// registration, or when it is neither a standard type the Device lists
    /// in Protocol_Object_Types_Supported
    /// ([`SUPPORTED_OBJECT_TYPES`](bacnet_objects::device::SUPPORTED_OBJECT_TYPES))
    /// nor a proprietary type from 128 to 1023. Registering a proprietary
    /// type doesn't add it to that list, which holds standard types only.
    ///
    /// A CreateObject for the type then goes through the same steps as one
    /// for a built-in type: the server's policy decides it, the identifier
    /// and instance checks run, `constructor` builds the object, the
    /// request's initial values are applied as WriteProperty applies them
    /// (or through [`BACnetObject::initialize_property`] for the object's
    /// creation-only properties), and the creation is audited. The PICS
    /// reports the type as createable.
    ///
    /// The constructor's answer:
    /// - An object whose identifier isn't the one it was given is refused
    ///   with OBJECT / DYNAMIC_CREATION_NOT_SUPPORTED and dropped, and so is
    ///   a panic (with unwind builds); nothing is added.
    /// - An [`Error::protocol`] refusal goes back to the client with its
    ///   class and code, naming no initial value. Any other error is
    ///   answered OBJECT / DYNAMIC_CREATION_NOT_SUPPORTED.
    pub fn register<F>(&mut self, object_type: ObjectType, constructor: F) -> Result<(), Error>
    where
        F: Fn(ObjectIdentifier, &str) -> Result<Box<dyn BACnetObject>, Error>
            + Send
            + Sync
            + 'static,
    {
        let raw = object_type.to_raw();
        let listed = bacnet_objects::device::SUPPORTED_OBJECT_TYPES.contains(&object_type);
        if !listed && !(128..=1023).contains(&raw) {
            return Err(registration_error(&format!(
                "{object_type} is neither a type the Device lists nor a proprietary type"
            )));
        }
        if self.by_type.contains_key(&raw) {
            return Err(registration_error(&format!(
                "{object_type} already has a constructor"
            )));
        }
        self.by_type.insert(raw, Arc::new(constructor));
        Ok(())
    }

    /// Whether `object_type` has a registered constructor.
    pub fn contains(&self, object_type: ObjectType) -> bool {
        self.by_type.contains_key(&object_type.to_raw())
    }

    /// The registered types, in ascending order.
    pub fn object_types(&self) -> impl Iterator<Item = ObjectType> + '_ {
        self.by_type.keys().map(|raw| ObjectType::from_raw(*raw))
    }

    pub(crate) fn get(&self, object_type: ObjectType) -> Option<&ObjectConstructor> {
        self.by_type.get(&object_type.to_raw())
    }
}

impl std::fmt::Debug for ObjectConstructors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.object_types()).finish()
    }
}

fn registration_error(message: &str) -> Error {
    Error::Encoding(format!("invalid object constructor: {message}"))
}

/// Build the object `oid` names through an application `constructor`, as
/// [`ObjectConstructors::register`] describes.
pub(crate) fn construct(
    constructor: &ObjectConstructor,
    oid: ObjectIdentifier,
    name: &str,
) -> Result<Box<dyn BACnetObject>, Error> {
    let not_created = || {
        Error::protocol(
            ErrorClass::OBJECT,
            ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED,
        )
    };
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| constructor(oid, name)))
        .map_err(|_| {
        tracing::warn!(%oid, "a CreateObject constructor panicked");
        not_created()
    })?;
    match built {
        Ok(object) if object.object_identifier() == oid => Ok(object),
        Ok(object) => {
            tracing::warn!(
                %oid,
                built = %object.object_identifier(),
                "a CreateObject constructor built another object"
            );
            Err(not_created())
        }
        Err(
            Error::Remote(bacnet_types::error::RemoteError::Error { class, code })
            | Error::Structured { class, code, .. },
        ) => Err(Error::protocol(class, code)),
        Err(_) => Err(not_created()),
    }
}

macro_rules! object_constructor_builder {
    () => {
        /// Make `object_type` creatable through `constructor`; see
        /// [`ObjectConstructors::register`], whose refusals this returns.
        pub fn object_constructor<F>(
            mut self,
            object_type: ObjectType,
            constructor: F,
        ) -> Result<Self, Error>
        where
            F: Fn(ObjectIdentifier, &str) -> Result<Box<dyn BACnetObject>, Error>
                + Send
                + Sync
                + 'static,
        {
            self.config
                .object_constructors
                .register(object_type, constructor)?;
            Ok(self)
        }
    };
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    object_constructor_builder!();
}

impl BipServerBuilder {
    object_constructor_builder!();
}

#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    object_constructor_builder!();
}

#[cfg(test)]
#[path = "object_constructors_tests.rs"]
pub(super) mod tests;
