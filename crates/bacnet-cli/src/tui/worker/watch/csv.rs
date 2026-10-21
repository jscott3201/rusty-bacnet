//! CSV logging of watched values to the profile workspace.
//!
//! A thread of its own does the file writes, fed through a bounded channel,
//! so the worker's loop never blocks on the disk. A line that finds the
//! channel full is dropped and counted.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

/// Lines waiting for the writer thread.
const QUEUE: usize = 4_096;

/// The header line. `device` is the instance alone and `network` the
/// remote network of a routed device, empty on this network: added last
/// so the earlier columns keep their places.
pub(crate) const HEADER: &str = "unix_ms,device,object,property,value,status_flags,source,network";

/// An open log.
pub(crate) struct CsvLog {
    tx: Option<SyncSender<String>>,
    thread: Option<JoinHandle<()>>,
    /// The file.
    pub(crate) path: PathBuf,
}

/// Quote a field when it holds a comma, a quote or a line break.
pub(crate) fn field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// Milliseconds since the Unix epoch, now.
pub(crate) fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
}

impl CsvLog {
    /// Create `watch-<unix ms>.csv` in `dir` and start its writer.
    pub(crate) fn open(dir: &Path) -> Result<Self, String> {
        let path = dir.join(format!("watch-{}.csv", unix_ms()));
        // A short local create; the writes go to the thread.
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        let (tx, rx) = sync_channel::<String>(QUEUE);
        let thread = std::thread::Builder::new()
            .name("bacnet-watch-csv".into())
            .spawn(move || write_lines(file, rx.iter()))
            .map_err(|e| format!("cannot start the CSV writer: {e}"))?;
        let log = Self {
            tx: Some(tx),
            thread: Some(thread),
            path,
        };
        log.line(HEADER.to_string());
        Ok(log)
    }

    /// Queue one line; false when it was dropped.
    pub(crate) fn line(&self, line: String) -> bool {
        match self.tx.as_ref().map(|tx| tx.try_send(line)) {
            Some(Ok(())) => true,
            Some(Err(TrySendError::Full(_) | TrySendError::Disconnected(_))) | None => false,
        }
    }

    /// Close the channel and wait, off the runtime's threads, for the
    /// writer to flush and end.
    pub(crate) async fn close(mut self) {
        drop(self.tx.take());
        if let Some(thread) = self.thread.take() {
            let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        }
    }
}

impl Drop for CsvLog {
    /// A log dropped without [`CsvLog::close`] (a session future dropped
    /// mid-way) still flushes: close the channel and wait for the writer,
    /// which has at most [`QUEUE`] short lines left.
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn write_lines(file: File, lines: impl Iterator<Item = String>) {
    let mut out = BufWriter::new(file);
    for line in lines {
        if writeln!(out, "{line}").is_err() {
            break;
        }
    }
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lines_reach_the_file_with_awkward_fields_quoted() {
        let dir = tempfile::tempdir().unwrap();
        let log = CsvLog::open(dir.path()).unwrap();
        assert!(log.line(format!("1,100,{},x", field("a \"b\", c"))));
        let path = log.path.clone();
        log.close().await;
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(text, format!("{HEADER}\n1,100,\"a \"\"b\"\", c\",x\n"));
    }
}
