//! Complete ordered step reporting for the clients scenario.
use super::client_diagnostics::ClientsDiagnostics;
use super::client_measure::StepReport;
use super::output::DegradationPoint;
use std::future::Future;
use std::time::{Duration, Instant};

pub struct ClientsRun {
    pub curve: Vec<DegradationPoint>,
    pub diagnostics: ClientsDiagnostics,
}

pub async fn run_steps<F, Fut>(
    object_count: u32,
    duration_secs: u64,
    steps: &[u64],
    mut step: F,
) -> ClientsRun
where
    F: FnMut(usize, u64, Duration) -> Fut,
    Fut: Future<Output = StepReport>,
{
    let duration = Duration::from_secs(duration_secs);
    let invalid = object_count == 0
        || object_count > 0x3f_ffff
        || duration.is_zero()
        || Instant::now().checked_add(duration).is_none()
        || steps.is_empty()
        || steps
            .iter()
            .any(|count| *count == 0 || usize::try_from(*count).is_err());
    let mut run = ClientsRun {
        curve: Vec::new(),
        diagnostics: ClientsDiagnostics {
            valid: false, expected_steps: steps.to_vec(), steps: Vec::new(),
            invalid_arguments: invalid.then(|| "objects, duration and client counts must be nonzero and representable; steps must not be empty".into()),
        },
    };
    if !invalid {
        for (index, &count) in steps.iter().enumerate() {
            eprintln!("--- {count} clients ---");
            let report = step(index, count, duration).await;
            if let Some(point) = report.point {
                point.log();
                run.curve.push(point);
            }
            run.diagnostics.steps.push(report.diagnostics);
        }
    }
    run.diagnostics.finish();
    run
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn invalid_arguments_never_start_a_step() {
        for (objects, duration, steps) in [
            (0, 1, vec![1]),
            (1, 0, vec![1]),
            (1, 1, vec![]),
            (1, 1, vec![0]),
        ] {
            let run = run_steps(objects, duration, &steps, |_, _, _| async {
                panic!("invalid run started setup")
            })
            .await;
            assert!(!run.diagnostics.valid);
            assert!(run.curve.is_empty());
            assert!(run.diagnostics.invalid_arguments.is_some());
        }
    }
    #[tokio::test]
    async fn failed_setup_remains_visible_and_does_not_erase_later_steps() {
        let run = run_steps(1, 1, &[2, 3], |index, count, _| async move {
            let mut report = StepReport::new(index, count);
            report.diagnostics.fail("setup", None, "injected");
            report.finish();
            report
        })
        .await;
        assert!(!run.diagnostics.valid);
        assert_eq!(run.diagnostics.steps.len(), 2);
        assert_eq!(run.diagnostics.steps[1].requested_clients, 3);
        let json = serde_json::to_value(run.diagnostics).unwrap();
        assert_eq!(json["valid"], false);
        assert_eq!(json["steps"][0]["lifecycle_failures"][0]["stage"], "setup");
    }
}
