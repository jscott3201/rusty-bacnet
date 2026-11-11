//! The arguments of `bacnet export` (#2436), out of `args.rs` for its size
//! cap.

use std::path::PathBuf;

use clap::Subcommand;

/// What `bacnet export` writes.
#[derive(Debug, Subcommand)]
pub(crate) enum ExportFormat {
    /// EXPERIMENTAL: the device's analog, binary and multi-state objects and
    /// integer values as a proposed ASHRAE Standard 223 point list, in
    /// Turtle.
    ///
    /// One 223 property per object, each with a BACnet external reference
    /// (device, object, name, property), QUDT units where they map, and the
    /// states of multi-state objects. A point list only: no equipment,
    /// connections or sensors. The 223 draft and this output can change
    /// between releases. Reads the device name, its object list, then one
    /// request per exported object; a target given by address costs one
    /// more, a read of its Device object to learn the instance. Stdout
    /// carries only the Turtle; a summary goes to stderr. --format and
    /// --json don't apply.
    #[command(name = "223-experimental")]
    S223 {
        /// Target device (address, instance, or routed dnet:instance).
        target: String,
        /// Base IRI of the generated nodes: an ASCII absolute IRI ending in
        /// / or #. A device's nodes go under the base followed by
        /// device-INSTANCE/.
        #[arg(long, value_name = "IRI",
              default_value = bacnet_ops::export::s223::DEFAULT_BASE_IRI)]
        base_iri: String,
        /// Object types to export, comma-separated (default: ai, ao, av,
        /// bi, bo, bv, msi, mso, msv and integer-value).
        #[arg(long, value_name = "TYPES", value_delimiter = ',')]
        types: Vec<String>,
        /// Command priority (1-16) to name in each commandable point's
        /// reference.
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=16))]
        write_priority: Option<u8>,
        /// Write the Turtle to this new file instead of stdout; an existing
        /// file is refused.
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
}
