//! Test runner — orchestrates test selection, execution, and result collection.

use std::time::{Duration, Instant};

use tokio::time::timeout;

use crate::engine::context::TestContext;
use crate::engine::registry::TestRegistry;
use crate::engine::selector::{TestFilter, TestSelector};
use crate::report::model::*;

/// Configuration for a test run.
pub struct RunConfig {
    pub filter: TestFilter,
    pub fail_fast: bool,
    pub default_timeout: Duration,
    pub dry_run: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            filter: TestFilter::default(),
            fail_fast: false,
            default_timeout: Duration::from_secs(30),
            dry_run: false,
        }
    }
}

/// Runs selected tests and collects results.
pub struct TestRunner {
    registry: TestRegistry,
}

impl TestRunner {
    pub fn new(registry: TestRegistry) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> &TestRegistry {
        &self.registry
    }

    /// Run all selected tests against the IUT via TestContext.
    pub async fn run(&self, ctx: &mut TestContext, config: &RunConfig) -> TestRun {
        let start = Instant::now();
        let selected = TestSelector::select(&self.registry, ctx.capabilities(), &config.filter);

        let total_selected = selected.len();
        let mut results: Vec<TestResult> = Vec::with_capacity(total_selected);

        for test in &selected {
            if config.dry_run {
                results.push(TestResult {
                    id: test.id.to_string(),
                    name: test.name.to_string(),
                    reference: test.reference.to_string(),
                    status: TestStatus::Skip {
                        reason: "dry run".into(),
                    },
                    steps: Vec::new(),
                    duration: Duration::ZERO,
                    notes: Vec::new(),
                });
                continue;
            }

            ctx.reset_steps();
            let test_timeout = test.timeout.unwrap_or(config.default_timeout);
            let test_start = Instant::now();

            let status = match timeout(test_timeout, (test.run)(ctx)).await {
                Ok(Ok(())) => TestStatus::Pass,
                Ok(Err(failure)) => TestStatus::Fail {
                    message: failure.message,
                    step: failure.step,
                },
                Err(_) => TestStatus::Error {
                    message: format!("Test timed out after {test_timeout:?}"),
                },
            };

            let result = TestResult {
                id: test.id.to_string(),
                name: test.name.to_string(),
                reference: test.reference.to_string(),
                status: status.clone(),
                steps: ctx.take_steps(),
                duration: test_start.elapsed(),
                notes: Vec::new(),
            };

            let failed = matches!(
                result.status,
                TestStatus::Fail { .. } | TestStatus::Error { .. }
            );
            results.push(result);

            if config.fail_fast && failed {
                break;
            }
        }

        let summary = Summary::from_results(&results);

        TestRun {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now(),
            duration: start.elapsed(),
            iut: ctx.iut_info(),
            transport: ctx.transport_info(),
            mode: ctx.test_mode(),
            suites: vec![TestSuiteResult {
                section: "all".into(),
                name: "BTL Test Run".into(),
                tests: results,
                duration: start.elapsed(),
                summary: summary.clone(),
            }],
            capture_file: None,
            summary,
        }
    }
}
