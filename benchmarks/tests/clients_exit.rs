//! Invalid clients runs must remain machine-readable failures.
use std::process::Command;

#[test]
fn invalid_clients_arguments_emit_one_json_document_and_fail() {
    for arguments in [["--duration", "0"], ["--objects", "0"], ["--steps", "0"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_stress-test"))
            .arg("clients")
            .args(arguments)
            .output()
            .expect("start stress-test");
        assert_eq!(output.status.code(), Some(1));
        let result: serde_json::Value = serde_json::from_slice(&output.stdout)
            .expect("stdout must contain exactly one JSON document");
        assert_eq!(result["clients_diagnostics"]["valid"], false);
        assert!(result["clients_diagnostics"]["invalid_arguments"].is_string());
        assert_eq!(result["results"]["total_requests"], 0);
        assert_eq!(
            result["results"]["degradation_curve"],
            serde_json::json!([])
        );
    }
}
