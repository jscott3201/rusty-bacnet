use super::*;

#[test]
fn failures_merge_without_losing_typed_reasons_or_latencies() {
    let mut a = Failures::default();
    a.record(FailureKind::Abort { reason: 1 }, Duration::from_secs(5));
    a.record(FailureKind::Timeout, Duration::from_secs(20));
    let mut b = Failures::default();
    b.record(FailureKind::Abort { reason: 1 }, Duration::from_secs(2));
    b.record(FailureKind::Reject { reason: 1 }, Duration::from_millis(10));
    a.merge(&b);
    assert_eq!(a.count, 4);
    assert_eq!(a.accounted(), 4);
    assert_eq!(a.buckets.len(), 3);
    assert_eq!(a.buckets[0].count, 2);
    assert_eq!(a.elapsed_ns, Duration::from_millis(27010).as_nanos());
    assert_eq!(a.max_ns, Duration::from_secs(20).as_nanos());
}

#[test]
fn arbitrary_protocol_codes_are_bounded_and_overflow_is_counted() {
    let mut failures = Failures::default();
    for code in 0..1000 {
        failures.record(FailureKind::Protocol { class: 2, code }, Duration::ZERO);
    }
    assert_eq!(failures.buckets.len(), 64);
    assert_eq!(failures.overflow, 936);
    assert_eq!(failures.accounted(), 1000);
}

fn complete() -> StepDiagnostics {
    let mut step = StepDiagnostics::new(0, 2);
    step.started_clients = 2;
    step.completed_clients = vec![1, 0];
    step
}

#[test]
fn missing_and_duplicate_clients_cannot_look_like_zero_errors() {
    for clients in [vec![0], vec![0, 0], vec![0, 2]] {
        let mut step = complete();
        step.completed_clients = clients;
        step.finish(0);
        assert!(!step.valid, "missing/duplicate client accepted");
    }
}

#[test]
fn setup_join_and_stop_failures_invalidate_even_with_all_clients() {
    for stage in ["setup", "join", "client_stop", "server_stop"] {
        let mut step = complete();
        step.fail(stage, None, "injected lifecycle failure");
        step.finish(0);
        assert!(!step.valid, "{stage} was ignored");
    }
}

#[test]
fn partial_start_and_unclassified_errors_are_invalid() {
    let mut step = complete();
    step.started_clients = 1;
    step.finish(0);
    assert!(!step.valid);
    let mut step = complete();
    step.finish(1);
    assert!(!step.valid);
}

#[test]
fn accounted_request_errors_do_not_invalidate_measurement() {
    let mut step = complete();
    step.failures
        .record(FailureKind::Timeout, Duration::from_secs(5));
    step.finish(1);
    assert!(step.valid);
}

#[test]
fn every_requested_step_must_have_its_own_report() {
    let mut run = ClientsDiagnostics {
        valid: false,
        expected_steps: vec![2, 2],
        steps: vec![complete()],
        invalid_arguments: None,
    };
    run.steps[0].finish(0);
    run.finish();
    assert!(!run.valid);
    run.steps.push(complete());
    run.steps[1].finish(0);
    run.finish();
    assert!(!run.valid, "duplicate step index accepted");
    run.steps[1].step_index = 1;
    run.finish();
    assert!(run.valid);
}

#[test]
fn empty_or_invalid_run_is_never_valid() {
    let mut run = ClientsDiagnostics {
        valid: false,
        expected_steps: vec![],
        steps: vec![],
        invalid_arguments: None,
    };
    run.finish();
    assert!(!run.valid);
    run.expected_steps.push(2);
    let mut step = complete();
    step.finish(0);
    run.steps.push(step);
    run.invalid_arguments = Some("zero duration".into());
    run.finish();
    assert!(!run.valid);
}
