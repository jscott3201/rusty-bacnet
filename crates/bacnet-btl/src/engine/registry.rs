//! Test definition registry — stores all BTL test definitions with metadata.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::engine::context::TestContext;
use crate::iut::capabilities::IutCapabilities;
use crate::report::model::TestFailure;

/// A BTL Test Plan section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    BasicFunctionality,
    Objects,
    DataSharing,
    AlarmAndEvent,
    Scheduling,
    Trending,
    DeviceManagement,
    DataLinkLayer,
    NetworkManagement,
    Gateway,
    NetworkSecurity,
    AuditReporting,
    WebServices,
}

impl Section {
    /// BTL Test Plan section number.
    pub fn number(&self) -> u8 {
        match self {
            Self::BasicFunctionality => 2,
            Self::Objects => 3,
            Self::DataSharing => 4,
            Self::AlarmAndEvent => 5,
            Self::Scheduling => 6,
            Self::Trending => 7,
            Self::DeviceManagement => 8,
            Self::DataLinkLayer => 9,
            Self::NetworkManagement => 10,
            Self::Gateway => 11,
            Self::NetworkSecurity => 12,
            Self::AuditReporting => 13,
            Self::WebServices => 14,
        }
    }

    /// Parse from a string like "2", "3", "10".
    pub fn from_number(n: &str) -> Option<Self> {
        match n {
            "2" => Some(Self::BasicFunctionality),
            "3" => Some(Self::Objects),
            "4" => Some(Self::DataSharing),
            "5" => Some(Self::AlarmAndEvent),
            "6" => Some(Self::Scheduling),
            "7" => Some(Self::Trending),
            "8" => Some(Self::DeviceManagement),
            "9" => Some(Self::DataLinkLayer),
            "10" => Some(Self::NetworkManagement),
            "11" => Some(Self::Gateway),
            "12" => Some(Self::NetworkSecurity),
            "13" => Some(Self::AuditReporting),
            "14" => Some(Self::WebServices),
            _ => None,
        }
    }
}

impl fmt::Display for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::BasicFunctionality => "Basic BACnet Functionality",
            Self::Objects => "Objects",
            Self::DataSharing => "Data Sharing BIBBs",
            Self::AlarmAndEvent => "Alarm and Event Management",
            Self::Scheduling => "Scheduling",
            Self::Trending => "Trending",
            Self::DeviceManagement => "Device Management",
            Self::DataLinkLayer => "Data Link Layer",
            Self::NetworkManagement => "Network Management",
            Self::Gateway => "Gateway",
            Self::NetworkSecurity => "Network Security",
            Self::AuditReporting => "Audit Reporting",
            Self::WebServices => "BACnet Web Services",
        };
        write!(f, "{} - {}", self.number(), name)
    }
}

/// A capability the IUT might or might not support.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Capability {
    /// A specific service by Protocol_Services_Supported bit position.
    Service(u8),
    /// A specific object type.
    ObjectType(u32),
    Segmentation,
    Cov,
    IntrinsicReporting,
    CommandPrioritization,
    WritableOutOfService,
    Transport(TransportRequirement),
    MultiNetwork,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TransportRequirement {
    Bip,
    Bip6,
    Mstp,
    Sc,
}

/// When a test can be skipped.
pub enum Conditionality {
    /// Must always be executed.
    MustExecute,
    /// Skip if IUT doesn't support this capability.
    RequiresCapability(Capability),
    /// Skip if IUT protocol revision is below this value.
    MinProtocolRevision(u16),
    /// Custom predicate evaluated against IUT capabilities.
    Custom(fn(&IutCapabilities) -> bool),
}

/// A single BTL test definition.
pub struct TestDef {
    /// Unique ID matching BTL Test Plan (e.g., "2.1.1").
    pub id: &'static str,
    /// Human-readable test name.
    pub name: &'static str,
    /// BTL/135.1 reference (e.g., "135.1-2025 - 13.4.3 - Invalid Tag").
    pub reference: &'static str,
    /// BTL Test Plan section.
    pub section: Section,
    /// Tags for filtering (e.g., "basic", "negative", "cov").
    pub tags: &'static [&'static str],
    /// When this test can be skipped.
    pub conditionality: Conditionality,
    /// Per-test timeout override (None = use runner default of 30s).
    pub timeout: Option<Duration>,
    /// The async test function.
    #[allow(clippy::type_complexity)]
    pub run: for<'a> fn(
        &'a mut TestContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), TestFailure>> + 'a>>,
}

/// Registry of all BTL test definitions.
pub struct TestRegistry {
    tests: Vec<TestDef>,
}

impl TestRegistry {
    pub fn new() -> Self {
        Self { tests: Vec::new() }
    }

    pub fn add(&mut self, test: TestDef) {
        self.tests.push(test);
    }

    pub fn tests(&self) -> &[TestDef] {
        &self.tests
    }

    pub fn len(&self) -> usize {
        self.tests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tests.is_empty()
    }

    /// Find a test by ID.
    pub fn find(&self, id: &str) -> Option<&TestDef> {
        self.tests.iter().find(|t| t.id == id)
    }

    /// Get all tests in a given section.
    pub fn by_section(&self, section: Section) -> Vec<&TestDef> {
        self.tests.iter().filter(|t| t.section == section).collect()
    }

    /// Get all tests with a given tag.
    pub fn by_tag(&self, tag: &str) -> Vec<&TestDef> {
        self.tests
            .iter()
            .filter(|t| t.tags.contains(&tag))
            .collect()
    }
}

impl Default for TestRegistry {
    fn default() -> Self {
        Self::new()
    }
}
