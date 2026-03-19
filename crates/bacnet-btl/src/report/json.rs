//! JSON report output.

use std::path::Path;

use crate::report::model::TestRun;

/// Serialize a test run to a JSON string.
pub fn to_json_string(run: &TestRun) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(run)
}

/// Save a test run as JSON to a file.
pub fn save_json(run: &TestRun, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let json = to_json_string(run)?;
    std::fs::write(path, json)?;
    Ok(())
}
