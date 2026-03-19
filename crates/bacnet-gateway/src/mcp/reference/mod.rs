//! BACnet reference knowledge base for MCP resources.
//!
//! Static reference material compiled into the binary. Teaches LLMs
//! about BACnet concepts, object types, properties, and troubleshooting.

pub mod content;

use rmcp::model::{Annotated, RawResource, RawResourceTemplate, Resource, ResourceTemplate};

/// All reference resource URIs and their metadata.
pub fn reference_resources() -> Vec<Resource> {
    vec![
        resource(
            "bacnet://reference/object-types",
            "BACnet Object Types",
            "Index of all 65 BACnet object types with name, category, and purpose",
        ),
        resource(
            "bacnet://reference/properties",
            "BACnet Properties",
            "Common BACnet properties: present-value, status-flags, reliability, out-of-service, event-state, priority-array",
        ),
        resource(
            "bacnet://reference/units",
            "BACnet Engineering Units",
            "All ~256 BACnet engineering units with descriptions",
        ),
        resource(
            "bacnet://reference/errors",
            "BACnet Error Codes",
            "Error classes and codes with common causes and next steps",
        ),
        resource(
            "bacnet://reference/reliability",
            "BACnet Reliability Values",
            "Reliability enum values: meaning, when they occur, how to clear",
        ),
        resource(
            "bacnet://reference/priority-array",
            "BACnet Priority Array",
            "16-level command priority scheme: what each level is for, relinquish-default, common pitfalls",
        ),
        resource(
            "bacnet://reference/networking",
            "BACnet Networking Guide",
            "Conceptual guide: networks, routers, BBMDs, foreign devices, broadcast domains",
        ),
        resource(
            "bacnet://reference/services",
            "BACnet Services",
            "When to use each service: ReadProperty vs ReadPropertyMultiple, COV vs polling, confirmed vs unconfirmed",
        ),
        resource(
            "bacnet://reference/troubleshooting",
            "BACnet Troubleshooting",
            "Common problem patterns, diagnostic steps, and resolution guides",
        ),
    ]
}

/// Live state resource metadata.
pub fn state_resources() -> Vec<Resource> {
    vec![
        resource(
            "bacnet://state/devices",
            "Discovered Devices",
            "Current device table — discovered devices with instance, vendor, MAC",
        ),
        resource(
            "bacnet://state/local-objects",
            "Local Objects",
            "Objects in the gateway's local BACnet database",
        ),
        resource(
            "bacnet://state/config",
            "Gateway Configuration",
            "Current gateway configuration (sanitized, no secrets)",
        ),
    ]
}

/// Template for per-object-type drill-down.
pub fn reference_templates() -> Vec<ResourceTemplate> {
    vec![Annotated::new(
        RawResourceTemplate::new(
            "bacnet://reference/object-types/{type}",
            "BACnet Object Type Detail",
        )
        .with_description(
            "Detailed reference for a specific BACnet object type: purpose, key properties, common configurations, troubleshooting",
        )
        .with_mime_type("text/plain"),
        None,
    )]
}

/// Look up reference content by URI.
pub fn read_reference(uri: &str) -> Option<String> {
    match uri {
        "bacnet://reference/object-types" => Some(content::OBJECT_TYPES_INDEX.to_string()),
        "bacnet://reference/properties" => Some(content::PROPERTIES.to_string()),
        "bacnet://reference/units" => Some(content::UNITS.to_string()),
        "bacnet://reference/errors" => Some(content::ERRORS.to_string()),
        "bacnet://reference/reliability" => Some(content::RELIABILITY.to_string()),
        "bacnet://reference/priority-array" => Some(content::PRIORITY_ARRAY.to_string()),
        "bacnet://reference/networking" => Some(content::NETWORKING.to_string()),
        "bacnet://reference/services" => Some(content::SERVICES.to_string()),
        "bacnet://reference/troubleshooting" => Some(content::TROUBLESHOOTING.to_string()),
        _ if uri.starts_with("bacnet://reference/object-types/") => {
            let type_name = uri.strip_prefix("bacnet://reference/object-types/")?;
            content::object_type_detail(type_name)
        }
        _ => None,
    }
}

fn resource(uri: &str, name: &str, description: &str) -> Resource {
    Annotated::new(
        RawResource::new(uri, name)
            .with_description(description)
            .with_mime_type("text/plain"),
        None,
    )
}
