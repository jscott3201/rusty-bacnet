//! Owned subprocesses; dropping a caller requests termination and reap.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
#[path = "../support/guide_process.rs"]
mod guide;
#[path = "../support/guide_retirement.rs"]
mod retirement;
pub use guide::{Process, WAIT, retired};

pub fn select_binary(external: Option<&OsStr>) -> Result<PathBuf, String> {
    let requested = external.unwrap_or_else(|| OsStr::new(env!("CARGO_BIN_EXE_guide-first-read")));
    let path = Path::new(requested)
        .canonicalize()
        .map_err(|e| format!("sample binary {requested:?}: {e}"))?;
    if !path.is_file() {
        return Err(format!("sample binary is not a file: {}", path.display()));
    }
    Ok(path)
}

#[allow(clippy::print_stderr)] // Record the exact executable selected for this acceptance run.
pub fn binary() -> PathBuf {
    let external = std::env::var_os("BACNET_FIRST_READ_BINARY");
    let path = select_binary(external.as_deref()).expect("resolve selected first-read executable");
    eprintln!(
        "first-read executable: {} (external={})",
        path.display(),
        external.is_some()
    );
    path
}
