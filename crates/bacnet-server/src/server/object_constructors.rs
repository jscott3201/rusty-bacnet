//! The application's own CreateObject constructors (#2173), and the
//! type-level creation and deletion answers that go with them (#2179).

use super::*;
use bacnet_objects::creation::{Creatable, CreationOnly};
use bacnet_objects::traits::BACnetObject;
use std::collections::{BTreeMap, BTreeSet};

/// A registered constructor with its object type erased. It runs
/// synchronously with the object database write-locked; see
/// [`ObjectConstructors::register`].
pub(crate) type ObjectConstructor =
    Arc<dyn Fn(ObjectIdentifier, &str) -> Result<Box<dyn BACnetObject>, Error> + Send + Sync>;

/// One registered type: its constructor and its creation-only answers.
#[derive(Clone)]
pub(crate) struct Registration {
    pub(crate) constructor: ObjectConstructor,
    pub(crate) creation_only: CreationOnly,
}

/// The object types an application makes creatable over the network, each
/// with its constructor, and the types it keeps from DeleteObject, set in
/// [`ServerConfig::object_constructors`] or through a server builder's
/// `object_constructor`.
///
/// CreateObject looks here before the built-in table,
/// [`bacnet_objects::creation`], so a registration may stand in for a
/// built-in type's constructor, for example to give new Analog Values
/// site-specific defaults. The registry is part of the configuration the
/// server starts with and doesn't change while it runs.
///
/// Creating and deleting are answered by object type, never by asking an
/// object: [`creates`](Self::creates), [`creation_only`](Self::creation_only)
/// and [`deletes`](Self::deletes) are what the CreateObject and
/// DeleteObject handlers and the PICS read.
#[derive(Clone, Default)]
pub struct ObjectConstructors {
    by_type: BTreeMap<u32, Registration>,
    kept: BTreeSet<u32>,
}

impl ObjectConstructors {
    /// An empty registry: CreateObject builds only the built-in types.
    pub fn new() -> Self {
        Self::default()
    }

    /// Make `object_type` creatable through `constructor`.
    ///
    /// Refused, leaving the registry unchanged, when the type already has a
    /// registration, when it is Device or Network Port (they model the node
    /// itself and its ports, so a client never creates one), or when it is
    /// neither a standard type the Device lists in
    /// Protocol_Object_Types_Supported
    /// ([`SUPPORTED_OBJECT_TYPES`](bacnet_objects::device::SUPPORTED_OBJECT_TYPES))
    /// nor a proprietary type from 128 to 1023. Registering a proprietary
    /// type doesn't add it to that list, which holds standard types only.
    ///
    /// A CreateObject for the type then goes through the same steps as one
    /// for a built-in type: the server's policy decides it, the identifier
    /// and instance checks run, `constructor` builds the object, the
    /// request's initial values are applied as WriteProperty applies them
    /// (or through [`Creatable::initialize_property`] for `O`'s
    /// creation-only properties), and the creation is audited. The PICS
    /// reports the type as createable, with `O`'s creation-only properties,
    /// even before it has an instance. A built-in object type implements
    /// [`Creatable`] already, so a constructor returning, say, an
    /// [`AnalogValueObject`](bacnet_objects::analog::AnalogValueObject)
    /// keeps its creation-only Units; an application's own type implements
    /// it, with an empty `impl` when it has no creation-only property.
    ///
    /// Unlike the built-in table, a registered constructor doesn't see the
    /// request's property list, so it can't provision optional rows on
    /// request: an initial value for a property the object doesn't serve
    /// (an Analog or Binary Value's Audit_Level, say, when the constructor
    /// left the audit policy rows out) is refused as a WriteProperty to it
    /// would be, and the object is dropped. Build the rows the type should
    /// accept.
    ///
    /// The constructor's answer:
    /// - An object whose identifier isn't the one it was given is refused
    ///   with OBJECT / DYNAMIC_CREATION_NOT_SUPPORTED and dropped, and so is
    ///   a panic (with unwind builds); nothing is added. A caught panic still
    ///   runs the process's panic hook first, which by default prints to
    ///   stderr (and can disturb a terminal UI): don't panic to refuse.
    /// - An [`Error::protocol`] refusal goes back to the client with its
    ///   class and code, naming no initial value. Any other error is
    ///   answered OBJECT / DYNAMIC_CREATION_NOT_SUPPORTED.
    pub fn register<O, F>(&mut self, object_type: ObjectType, constructor: F) -> Result<(), Error>
    where
        O: Creatable,
        F: Fn(ObjectIdentifier, &str) -> Result<O, Error> + Send + Sync + 'static,
    {
        let raw = object_type.to_raw();
        if matches!(object_type, ObjectType::DEVICE | ObjectType::NETWORK_PORT) {
            return Err(registration_error(&format!(
                "{object_type} objects model the device itself and are never created"
            )));
        }
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
        let constructor: ObjectConstructor = Arc::new(move |oid, name| {
            constructor(oid, name).map(|object| Box::new(object) as Box<dyn BACnetObject>)
        });
        self.by_type.insert(
            raw,
            Registration {
                constructor,
                creation_only: CreationOnly::of::<O>(),
            },
        );
        Ok(())
    }

    /// Keep DeleteObject from removing objects of `object_type`: it then
    /// refuses them with OBJECT / OBJECT_DELETION_NOT_PERMITTED, and the
    /// PICS reports the type as not deleteable.
    ///
    /// Deleteability belongs to the type. Every type is deleteable but
    /// Device and Network Port
    /// ([`builtin_deleteable`](bacnet_objects::creation::builtin_deleteable))
    /// and the types kept here; use this for a type that models a fixed
    /// part of the device. Removing an object locally, through the
    /// database, isn't affected.
    pub fn keep_from_deletion(&mut self, object_type: ObjectType) {
        self.kept.insert(object_type.to_raw());
    }

    /// Whether `object_type` has a registered constructor.
    pub fn contains(&self, object_type: ObjectType) -> bool {
        self.by_type.contains_key(&object_type.to_raw())
    }

    /// Whether CreateObject builds `object_type`: it has a registered
    /// constructor, or an entry in the built-in table
    /// ([`bacnet_objects::creation`]). Createability belongs to the type,
    /// so this is the one answer: the CreateObject handler builds through
    /// the same two sources, in that order, and the PICS reports this.
    pub fn creates(&self, object_type: ObjectType) -> bool {
        self.contains(object_type)
            || bacnet_objects::creation::builtin_constructor(object_type).is_some()
    }

    /// What a new object of `object_type` takes only at creation: the
    /// registered type's, or else the built-in table's.
    pub fn creation_only(&self, object_type: ObjectType) -> CreationOnly {
        self.get(object_type).map_or_else(
            || bacnet_objects::creation::builtin_creation_only(object_type),
            |registration| registration.creation_only,
        )
    }

    /// Whether DeleteObject removes objects of `object_type`: the built-in
    /// answer, unless [`keep_from_deletion`](Self::keep_from_deletion) kept
    /// the type.
    pub fn deletes(&self, object_type: ObjectType) -> bool {
        !self.kept.contains(&object_type.to_raw())
            && bacnet_objects::creation::builtin_deleteable(object_type)
    }

    /// The registered types, in ascending order.
    pub fn object_types(&self) -> impl Iterator<Item = ObjectType> + '_ {
        self.by_type.keys().map(|raw| ObjectType::from_raw(*raw))
    }

    pub(crate) fn get(&self, object_type: ObjectType) -> Option<&Registration> {
        self.by_type.get(&object_type.to_raw())
    }
}

impl std::fmt::Debug for ObjectConstructors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectConstructors")
            .field("created", &self.object_types().collect::<Vec<_>>())
            .field(
                "kept_from_deletion",
                &self
                    .kept
                    .iter()
                    .map(|raw| ObjectType::from_raw(*raw))
                    .collect::<Vec<_>>(),
            )
            .finish()
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
        pub fn object_constructor<O, F>(
            mut self,
            object_type: ObjectType,
            constructor: F,
        ) -> Result<Self, Error>
        where
            O: Creatable,
            F: Fn(ObjectIdentifier, &str) -> Result<O, Error> + Send + Sync + 'static,
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
