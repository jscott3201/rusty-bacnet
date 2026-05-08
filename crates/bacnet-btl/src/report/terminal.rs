//! Colored terminal output for test results.

use owo_colors::OwoColorize;

use crate::report::model::*;

/// Print the run header banner.
pub fn print_run_header(iut: &IutInfo, transport: &TransportInfo, mode: &TestMode) {
    let mode_str = match mode {
        TestMode::SelfTestInProcess => "self-test (in-process)",
        TestMode::SelfTestSubprocess => "self-test (subprocess)",
        TestMode::SelfTestDocker => "self-test (docker)",
        TestMode::External => "external IUT",
    };
    println!();
    println!("{}", "BTL Compliance Test Run".bold());
    println!(
        "IUT: Device {} ({}) via {} [{}]",
        iut.device_instance, iut.vendor_name, transport.transport_type, mode_str
    );
    println!("{}", "═".repeat(72));
}

/// Print a single test result line.
pub fn print_test_result(result: &TestResult, verbose: bool) {
    let (marker, color_fn): (&str, fn(&str) -> String) = match &result.status {
        TestStatus::Pass => ("✓", |s| s.green().to_string()),
        TestStatus::Fail { .. } => ("✗", |s| s.red().to_string()),
        TestStatus::Skip { .. } => ("○", |s| s.yellow().to_string()),
        TestStatus::Manual { .. } => ("?", |s| s.blue().to_string()),
        TestStatus::Error { .. } => ("!", |s| s.red().bold().to_string()),
    };

    let duration_str = format!("{:.2}s", result.duration.as_secs_f64());
    let status_line = format!(
        "  {} {:6}  {:<50} {}",
        marker, result.id, result.name, duration_str
    );
    println!("{}", color_fn(&status_line));

    // Show failure details
    match &result.status {
        TestStatus::Fail { message, step } => {
            let step_str = step.map(|s| format!("Step {}: ", s)).unwrap_or_default();
            println!("           {}{}", step_str, message.red());
        }
        TestStatus::Skip { reason } if verbose => {
            println!("           SKIP: {}", reason.yellow());
        }
        TestStatus::Error { message } => {
            println!("           ERROR: {}", message.red().bold());
        }
        _ => {}
    }

    // Show step details in verbose mode
    if verbose && !result.steps.is_empty() {
        for step in &result.steps {
            let step_marker = if step.pass { "✓" } else { "✗" };
            let action_str = match &step.action {
                StepAction::Verify {
                    object, property, ..
                } => format!("VERIFY {object}.{property}"),
                StepAction::Write {
                    object, property, ..
                } => format!("WRITE {object}.{property}"),
                StepAction::Transmit { service, .. } => format!("TRANSMIT {service}"),
                StepAction::Receive { pdu_type } => format!("RECEIVE {pdu_type}"),
                StepAction::Make { description, .. } => format!("MAKE {description}"),
                StepAction::Wait { duration } => format!("WAIT {duration:?}"),
            };
            let step_line = format!(
                "    [{step_marker}] Step {}: {}",
                step.step_number, action_str
            );
            if step.pass {
                println!("    {}", step_line.dimmed());
            } else {
                println!("    {}", step_line.red());
                if let Some(ref actual) = step.actual {
                    println!("         Got: {}", actual.red());
                }
            }
        }
    }
}

/// Print the final summary.
pub fn print_summary(summary: &Summary) {
    println!("{}", "═".repeat(72));
    let total_line = format!(
        "TOTAL: {} tests — {} passed, {} failed, {} skipped, {} manual, {} errors",
        summary.total,
        summary.passed,
        summary.failed,
        summary.skipped,
        summary.manual,
        summary.errors
    );
    if summary.failed > 0 || summary.errors > 0 {
        println!("{}", total_line.red().bold());
    } else {
        println!("{}", total_line.green().bold());
    }
    println!("Duration: {:.1}s", summary.duration.as_secs_f64());
}

/// Print a complete test run.
pub fn print_test_run(run: &TestRun, verbose: bool) {
    print_run_header(&run.iut, &run.transport, &run.mode);

    for suite in &run.suites {
        for result in &suite.tests {
            print_test_result(result, verbose);
        }
    }

    println!();
    print_summary(&run.summary);
}
