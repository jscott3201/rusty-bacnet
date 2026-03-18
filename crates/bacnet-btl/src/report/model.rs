//! Test result reporting data model.
//!
//! All types here are serializable for JSON output and cloneable for
//! the reporter to hold copies while the runner continues.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A complete test run — top-level result container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRun {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    pub iut: IutInfo,
    pub transport: TransportInfo,
    pub mode: TestMode,
    pub suites: Vec<TestSuiteResult>,
    pub capture_file: Option<String>,
    pub summary: Summary,
}

/// Information about the IUT (Implementation Under Test).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IutInfo {
    pub device_instance: u32,
    pub vendor_name: String,
    pub vendor_id: u16,
    pub model_name: String,
    pub firmware_revision: String,
    pub protocol_revision: u16,
    pub address: String,
}

/// Information about the transport used for the test run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportInfo {
    pub transport_type: String,
    pub local_address: String,
    pub details: String,
}

/// How the test run was executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestMode {
    SelfTestInProcess,
    SelfTestSubprocess,
    SelfTestDocker,
    External,
}

/// Results grouped by BTL Test Plan section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSuiteResult {
    pub section: String,
    pub name: String,
    pub tests: Vec<TestResult>,
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    pub summary: Summary,
}

/// Result of a single BTL test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub id: String,
    pub name: String,
    pub reference: String,
    pub status: TestStatus,
    pub steps: Vec<StepResult>,
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    pub notes: Vec<String>,
}

/// Outcome of a test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestStatus {
    Pass,
    Fail { message: String, step: Option<u16> },
    Skip { reason: String },
    Manual { description: String },
    Error { message: String },
}

/// Result of a single step within a test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_number: u16,
    pub action: StepAction,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub pass: bool,
    pub timestamp: DateTime<Utc>,
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_apdu: Option<Vec<u8>>,
}

/// The kind of action a step performs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepAction {
    Transmit {
        service: String,
        details: String,
    },
    Receive {
        pdu_type: String,
    },
    Verify {
        object: String,
        property: String,
        value: String,
    },
    Write {
        object: String,
        property: String,
        value: String,
    },
    Make {
        description: String,
        method: String,
    },
    Wait {
        #[serde(with = "duration_serde")]
        duration: Duration,
    },
}

/// Aggregate pass/fail/skip counts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub manual: u32,
    pub errors: u32,
    #[serde(with = "duration_serde")]
    pub duration: Duration,
}

impl Summary {
    /// Build a summary from a list of test results.
    pub fn from_results(results: &[TestResult]) -> Self {
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut skipped = 0u32;
        let mut manual = 0u32;
        let mut errors = 0u32;
        let mut duration = Duration::ZERO;
        for r in results {
            match &r.status {
                TestStatus::Pass => passed += 1,
                TestStatus::Fail { .. } => failed += 1,
                TestStatus::Skip { .. } => skipped += 1,
                TestStatus::Manual { .. } => manual += 1,
                TestStatus::Error { .. } => errors += 1,
            }
            duration += r.duration;
        }
        Self {
            total: results.len() as u32,
            passed,
            failed,
            skipped,
            manual,
            errors,
            duration,
        }
    }
}

/// A test failure returned by test functions.
#[derive(Debug, Clone)]
pub struct TestFailure {
    pub message: String,
    pub step: Option<u16>,
}

impl TestFailure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            step: None,
        }
    }

    pub fn at_step(step: u16, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            step: Some(step),
        }
    }
}

impl std::fmt::Display for TestFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(step) = self.step {
            write!(f, "Step {}: {}", step, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for TestFailure {}

/// Serde helper for Duration (serialized as fractional seconds).
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_secs_f64().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = f64::deserialize(d)?;
        Ok(Duration::from_secs_f64(secs))
    }
}
