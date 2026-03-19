//! MAKE step dispatch — handles direct DB access, BACnet writes, and interactive prompts.

use bacnet_objects::database::ObjectDatabase;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

/// How a MAKE step should be executed.
pub enum MakeAction {
    /// Direct DB manipulation (self-test in-process only).
    /// The engine acquires a write lock on Arc<RwLock<ObjectDatabase>> before calling.
    Direct(Box<dyn FnOnce(&mut ObjectDatabase) + Send>),

    /// Try BACnet write first, fall back to interactive prompt on failure.
    WriteOrPrompt {
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        value: Vec<u8>,
        prompt: String,
    },

    /// Requires human interaction (power cycle, wire disconnect, etc.).
    ManualOnly(String),
}
