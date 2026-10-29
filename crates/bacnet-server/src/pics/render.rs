//! The text and Markdown renderings of a [`Pics`] document.

use super::device_text::{markdown_cell, text_line};
use super::{ObjectTypeSupport, Pics, PropertySupport};

const PROPERTY_CAPABILITIES_EXPLANATION: &str = "Property rows aggregate configured instances: a row or access flag means at least one instance supports it. Actual availability and access depend on the concrete object. Optional is the metadata conformance classification; a required declaration wins, and absent rows do not vote.";

/// How the text and Markdown PICS introduce a type's
/// [`ObjectTypeSupport::creation_only_properties`]: read-only to
/// WriteProperty as a whole, but a CreateObject initial value sets them.
const CREATION_ONLY_LABEL: &str = "Whole value set only by CreateObject";

/// A row's access as the text and Markdown PICS print it: the flags, and for
/// a row writes of another property change
/// ([`PropertySupport::written_through`]) a note saying so, since no write
/// naming the row itself is taken (#1443).
fn access_text(row: &PropertySupport) -> String {
    match row.written_through {
        Some(through) => format!(
            "{} (resized through {through}: a whole write or its size at index 0)",
            row.access
        ),
        None => row.access.to_string(),
    }
}

/// The type's creation-only properties, comma-separated, or `None` if it
/// has none.
fn creation_only_names(support: &ObjectTypeSupport) -> Option<String> {
    let names: Vec<_> = support
        .creation_only_properties
        .iter()
        .map(ToString::to_string)
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

impl Pics {
    /// Render the PICS as human-readable text per Annex A layout.
    pub fn generate_text(&self) -> String {
        let mut out = String::with_capacity(4096);

        out.push_str("=== BACnet Protocol Implementation Conformance Statement (PICS) ===\n");
        out.push_str("    Per ASHRAE 135-2020 Annex A\n\n");

        out.push_str("--- Vendor Information ---\n");
        out.push_str(&format!(
            "Vendor ID:                      {}\n",
            self.vendor_info.vendor_id
        ));
        out.push_str(&format!(
            "Vendor Name:                    {}\n",
            text_line(&self.vendor_info.vendor_name)
        ));
        out.push_str(&format!(
            "Model Name:                     {}\n",
            text_line(&self.vendor_info.model_name)
        ));
        out.push_str(&format!(
            "Firmware Revision:              {}\n",
            text_line(&self.vendor_info.firmware_revision)
        ));
        out.push_str(&format!(
            "Application Software Version:   {}\n",
            text_line(&self.vendor_info.application_software_version)
        ));
        out.push_str(&format!(
            "Protocol Version:               {}\n",
            self.vendor_info.protocol_version
        ));
        out.push_str(&format!(
            "Protocol Revision:              {}\n\n",
            self.vendor_info.protocol_revision
        ));

        out.push_str("--- BACnet Device Profile ---\n");
        out.push_str(&format!("Profile: {}\n\n", self.device_profile));

        out.push_str("--- Supported Object Types ---\n");
        out.push_str(PROPERTY_CAPABILITIES_EXPLANATION);
        out.push('\n');
        for ot in &self.supported_object_types {
            out.push_str(&format!(
                "\n  Object Type: {} (createable={}, deleteable={})\n",
                ot.object_type, ot.createable, ot.deleteable
            ));
            if let Some(names) = creation_only_names(ot) {
                out.push_str(&format!("  {CREATION_ONLY_LABEL}: {names}\n"));
            }
            out.push_str("  Properties:\n");
            for prop in &ot.supported_properties {
                out.push_str(&format!(
                    "    {:<40} {}\n",
                    prop.property_id.to_string(),
                    access_text(prop)
                ));
            }
        }
        out.push('\n');

        out.push_str("--- Supported Services ---\n");
        out.push_str(&format!(
            "  {:<45} {:>9} {:>9}\n",
            "Service", "Initiator", "Executor"
        ));
        out.push_str(&format!("  {:-<45} {:-<9} {:-<9}\n", "", "", ""));
        for svc in &self.supported_services {
            let init = if svc.initiator { "Yes" } else { "No" };
            let exec = if svc.executor { "Yes" } else { "No" };
            out.push_str(&format!(
                "  {:<45} {:>9} {:>9}\n",
                svc.service_name, init, exec
            ));
        }
        out.push('\n');

        out.push_str("--- Data Link Layer Support ---\n");
        for dl in &self.data_link_layers {
            out.push_str(&format!("  {dl}\n"));
        }
        out.push('\n');

        out.push_str("--- Network Layer Options ---\n");
        out.push_str(&format!(
            "  Router:         {}\n",
            self.network_layer.router
        ));
        out.push_str(&format!("  BBMD:           {}\n", self.network_layer.bbmd));
        out.push_str(&format!(
            "  Foreign Device: {}\n\n",
            self.network_layer.foreign_device
        ));

        out.push_str("--- Character Sets Supported ---\n");
        for cs in &self.character_sets {
            out.push_str(&format!("  {cs}\n"));
        }
        out.push('\n');

        if !self.special_functionality.is_empty() {
            out.push_str("--- Special Functionality ---\n");
            for sf in &self.special_functionality {
                out.push_str(&format!("  {sf}\n"));
            }
            out.push('\n');
        }

        out
    }

    /// Render the PICS as Markdown for documentation.
    pub fn generate_markdown(&self) -> String {
        let mut out = String::with_capacity(4096);

        out.push_str("# BACnet Protocol Implementation Conformance Statement (PICS)\n\n");
        out.push_str("*Per ASHRAE 135-2020 Annex A*\n\n");

        out.push_str("## Vendor Information\n\n");
        out.push_str("| Field | Value |\n");
        out.push_str("|-------|-------|\n");
        out.push_str(&format!("| Vendor ID | {} |\n", self.vendor_info.vendor_id));
        out.push_str(&format!(
            "| Vendor Name | {} |\n",
            markdown_cell(&self.vendor_info.vendor_name)
        ));
        out.push_str(&format!(
            "| Model Name | {} |\n",
            markdown_cell(&self.vendor_info.model_name)
        ));
        out.push_str(&format!(
            "| Firmware Revision | {} |\n",
            markdown_cell(&self.vendor_info.firmware_revision)
        ));
        out.push_str(&format!(
            "| Application Software Version | {} |\n",
            markdown_cell(&self.vendor_info.application_software_version)
        ));
        out.push_str(&format!(
            "| Protocol Version | {} |\n",
            self.vendor_info.protocol_version
        ));
        out.push_str(&format!(
            "| Protocol Revision | {} |\n\n",
            self.vendor_info.protocol_revision
        ));

        out.push_str("## BACnet Device Profile\n\n");
        out.push_str(&format!("**{}**\n\n", self.device_profile));

        out.push_str("## Supported Object Types\n\n");
        out.push_str(PROPERTY_CAPABILITIES_EXPLANATION);
        out.push_str("\n\n");
        for ot in &self.supported_object_types {
            out.push_str(&format!(
                "### {}\n\n- Createable: {}\n- Deleteable: {}\n",
                ot.object_type, ot.createable, ot.deleteable
            ));
            if let Some(names) = creation_only_names(ot) {
                out.push_str(&format!("- {CREATION_ONLY_LABEL}: {names}\n"));
            }
            out.push('\n');
            out.push_str("| Property | Access |\n");
            out.push_str("|----------|--------|\n");
            for prop in &ot.supported_properties {
                out.push_str(&format!(
                    "| {} | {} |\n",
                    prop.property_id,
                    access_text(prop)
                ));
            }
            out.push('\n');
        }

        out.push_str("## Supported Services\n\n");
        out.push_str("| Service | Initiator | Executor |\n");
        out.push_str("|---------|-----------|----------|\n");
        for svc in &self.supported_services {
            let init = if svc.initiator { "Yes" } else { "No" };
            let exec = if svc.executor { "Yes" } else { "No" };
            out.push_str(&format!("| {} | {} | {} |\n", svc.service_name, init, exec));
        }
        out.push('\n');

        out.push_str("## Data Link Layer Support\n\n");
        for dl in &self.data_link_layers {
            out.push_str(&format!("- {dl}\n"));
        }
        out.push('\n');

        out.push_str("## Network Layer Options\n\n");
        out.push_str("| Feature | Supported |\n");
        out.push_str("|---------|-----------|\n");
        out.push_str(&format!("| Router | {} |\n", self.network_layer.router));
        out.push_str(&format!("| BBMD | {} |\n", self.network_layer.bbmd));
        out.push_str(&format!(
            "| Foreign Device | {} |\n\n",
            self.network_layer.foreign_device
        ));

        out.push_str("## Character Sets Supported\n\n");
        for cs in &self.character_sets {
            out.push_str(&format!("- {cs}\n"));
        }
        out.push('\n');

        if !self.special_functionality.is_empty() {
            out.push_str("## Special Functionality\n\n");
            for sf in &self.special_functionality {
                out.push_str(&format!("- {sf}\n"));
            }
            out.push('\n');
        }

        out
    }
}
