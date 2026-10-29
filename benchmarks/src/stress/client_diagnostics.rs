//! Bounded request failures and explicit validity for the clients workload.
use serde::Serialize;
use std::time::Duration;

/// Typed protocol values are normalized by the version-specific adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailureKind {
    Timeout,
    Abort { reason: u8 },
    Reject { reason: u8 },
    Protocol { class: u32, code: u32 },
    Transport { io_kind: String },
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailureBucket {
    pub failure: FailureKind,
    pub count: u64,
}

/// Failed-call timings never enter the successful-operation histogram.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Failures {
    pub buckets: Vec<FailureBucket>,
    pub overflow: u64,
    pub count: u64,
    pub elapsed_ns: u128,
    pub max_ns: u128,
}

impl Failures {
    pub fn record(&mut self, failure: FailureKind, elapsed: Duration) {
        self.add(failure, 1);
        self.count += 1;
        self.elapsed_ns += elapsed.as_nanos();
        self.max_ns = self.max_ns.max(elapsed.as_nanos());
    }

    fn add(&mut self, failure: FailureKind, count: u64) {
        if let Some(bucket) = self.buckets.iter_mut().find(|b| b.failure == failure) {
            bucket.count += count;
        } else if self.buckets.len() < 64 {
            self.buckets.push(FailureBucket { failure, count });
        } else {
            self.overflow += count;
        }
    }

    pub fn merge(&mut self, other: &Self) {
        for bucket in &other.buckets {
            self.add(bucket.failure.clone(), bucket.count);
        }
        self.overflow += other.overflow;
        self.count += other.count;
        self.elapsed_ns += other.elapsed_ns;
        self.max_ns = self.max_ns.max(other.max_ns);
    }

    pub fn accounted(&self) -> u64 {
        self.overflow + self.buckets.iter().map(|b| b.count).sum::<u64>()
    }
}

#[derive(Debug, Serialize)]
pub struct LifecycleFailure {
    pub stage: &'static str,
    pub client: Option<usize>,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct StepDiagnostics {
    pub step_index: usize,
    pub requested_clients: u64,
    pub started_clients: usize,
    pub completed_clients: Vec<usize>,
    pub failures: Failures,
    pub lifecycle_failures: Vec<LifecycleFailure>,
    pub valid: bool,
    pub server_counters: Option<super::client_counters::CounterDelta>,
    pub client_counters: Vec<super::client_counters::CounterDelta>,
}

impl StepDiagnostics {
    pub fn new(step_index: usize, requested_clients: u64) -> Self {
        Self {
            step_index,
            requested_clients,
            started_clients: 0,
            completed_clients: Vec::new(),
            failures: Failures::default(),
            lifecycle_failures: Vec::new(),
            valid: false,
            server_counters: None,
            client_counters: Vec::new(),
        }
    }

    pub fn fail(&mut self, stage: &'static str, client: Option<usize>, message: impl ToString) {
        self.lifecycle_failures.push(LifecycleFailure {
            stage,
            client,
            message: message.to_string(),
        });
        self.valid = false;
    }

    pub fn finish(&mut self, errors: u64) {
        let mut indices = self.completed_clients.clone();
        indices.sort_unstable();
        let complete = self.requested_clients > 0
            && self.started_clients as u64 == self.requested_clients
            && indices.len() as u64 == self.requested_clients
            && indices
                .iter()
                .enumerate()
                .all(|(i, observed)| i == *observed);
        if !complete {
            self.fail(
                "accounting",
                None,
                "not every requested client completed exactly once",
            );
        }
        if errors != self.failures.count || errors != self.failures.accounted() {
            self.fail(
                "accounting",
                None,
                "request errors and classified failures differ",
            );
        }
        self.valid = self.lifecycle_failures.is_empty();
    }
}

#[derive(Debug, Serialize)]
pub struct ClientsDiagnostics {
    pub valid: bool,
    pub expected_steps: Vec<u64>,
    pub steps: Vec<StepDiagnostics>,
    pub invalid_arguments: Option<String>,
}

impl ClientsDiagnostics {
    pub fn finish(&mut self) {
        self.valid =
            self.invalid_arguments.is_none()
                && !self.expected_steps.is_empty()
                && self.steps.len() == self.expected_steps.len()
                && self.steps.iter().zip(&self.expected_steps).enumerate().all(
                    |(i, (step, count))| {
                        step.valid && step.step_index == i && step.requested_clients == *count
                    },
                );
    }
}

#[cfg(test)]
#[path = "client_diagnostics_tests.rs"]
mod tests;
