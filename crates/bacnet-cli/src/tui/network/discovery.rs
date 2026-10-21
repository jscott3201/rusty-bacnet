//! Who-Has and router discovery messages (#960): what the UI asks the
//! discovery worker to send, and what it hears back, as plain data with no
//! transport type. Unlike the BBMD reads, these work on every transport.

use bacnet_types::primitives::ObjectIdentifier;

/// UI to the discovery worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiscoveryCommand {
    /// Broadcast a Who-Has for the object with this name.
    WhoHas {
        /// The object name, compared exactly.
        name: String,
    },
    /// Broadcast a Who-Is-Router-To-Network for every network.
    WhoIsRouter,
}

/// One object an I-Have reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FoundRow {
    /// The device that has it.
    pub(crate) device: u32,
    /// The object.
    pub(crate) object: ObjectIdentifier,
    /// Its name.
    pub(crate) name: String,
    /// Where the I-Have came from, as a target the CLI reads back: the
    /// device's address, or for one a router passed it on from, its routed
    /// station (`2:0x05`), as the device table writes them.
    pub(crate) address: String,
    /// The router that passed the I-Have on, on the local network.
    pub(crate) via: Option<String>,
    /// The device's network, when a router passed the I-Have on.
    pub(crate) network: Option<u16>,
}

/// The discovery worker to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiscoveryEvent {
    /// An I-Have arrived, asked for or not.
    IHave(FoundRow),
    /// A router listed the networks it reaches.
    Router {
        /// Its address on the local network.
        router: String,
        /// The networks it listed.
        networks: Vec<u16>,
    },
    /// A Network-Number-Is announced the local network's number.
    NetworkNumber {
        /// The number.
        network: u16,
        /// Whether the sender had it configured rather than learned.
        configured: bool,
    },
    /// A router refused a message this client sent.
    Rejected {
        /// The router's address on the local network.
        router: String,
        /// The network the refused message was for.
        network: u16,
        /// Why, by name.
        reason: String,
    },
    /// A request could not be sent.
    Refused {
        /// The request.
        command: DiscoveryCommand,
        /// Why.
        reason: String,
    },
}
