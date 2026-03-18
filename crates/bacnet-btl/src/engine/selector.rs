//! Test selection and filtering — evaluates conditionality and user filters.

use bacnet_types::enums::ObjectType;

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::iut::capabilities::IutCapabilities;

/// Filters for narrowing which tests to run.
#[derive(Debug, Default)]
pub struct TestFilter {
    pub section: Option<String>,
    pub tag: Option<String>,
    pub test_id: Option<String>,
    pub search: Option<String>,
}

/// Selects which tests to run given IUT capabilities and user filters.
pub struct TestSelector;

impl TestSelector {
    pub fn select<'a>(
        registry: &'a TestRegistry,
        capabilities: &IutCapabilities,
        filter: &TestFilter,
    ) -> Vec<&'a TestDef> {
        registry
            .tests()
            .iter()
            .filter(|test| Self::matches_conditionality(test, capabilities))
            .filter(|test| Self::matches_filter(test, filter))
            .collect()
    }

    fn matches_conditionality(test: &TestDef, caps: &IutCapabilities) -> bool {
        match &test.conditionality {
            Conditionality::MustExecute => true,
            Conditionality::RequiresCapability(cap) => Self::has_capability(caps, cap),
            Conditionality::MinProtocolRevision(rev) => caps.protocol_revision >= *rev,
            Conditionality::Custom(f) => f(caps),
        }
    }

    fn has_capability(caps: &IutCapabilities, cap: &Capability) -> bool {
        match cap {
            Capability::Service(sc) => caps.services_supported.contains(sc),
            Capability::ObjectType(ot) => caps.object_types.contains(&ObjectType::from_raw(*ot)),
            Capability::Segmentation => caps.segmentation_supported != 3,
            Capability::Cov => caps.services_supported.contains(&5),
            Capability::IntrinsicReporting => caps
                .object_details
                .values()
                .any(|d| d.supports_intrinsic_reporting),
            Capability::CommandPrioritization => {
                caps.object_details.values().any(|d| d.commandable)
            }
            Capability::WritableOutOfService => caps
                .object_details
                .values()
                .any(|d| d.out_of_service_writable),
            Capability::Transport(_) => true,
            Capability::MultiNetwork => false,
        }
    }

    fn matches_filter(test: &TestDef, filter: &TestFilter) -> bool {
        if let Some(ref section) = filter.section {
            // Match by test ID prefix ("3.1" matches "3.1.1", "3.1.2", etc.)
            // or by section number ("3" matches all Section::Objects tests)
            if !test.id.starts_with(section.as_str()) {
                if let Some(s) = Section::from_number(section) {
                    if test.section != s {
                        return false;
                    }
                } else {
                    return false;
                }
            }
        }
        if let Some(ref tag) = filter.tag {
            if !test.tags.contains(&tag.as_str()) {
                return false;
            }
        }
        if let Some(ref test_id) = filter.test_id {
            if test.id != test_id.as_str() {
                return false;
            }
        }
        if let Some(ref search) = filter.search {
            let s = search.to_lowercase();
            if !test.name.to_lowercase().contains(&s) && !test.reference.to_lowercase().contains(&s)
            {
                return false;
            }
        }
        true
    }
}
