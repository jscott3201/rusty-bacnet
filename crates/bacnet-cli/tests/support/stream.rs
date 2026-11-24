//! A long-running command's stdout as JSON Lines (#2783): each line one
//! compact JSON object naming its kind in `type`, and nothing else.

use std::io::{BufRead, BufReader, Read};

use serde_json::Value;
use tokio::sync::mpsc;

/// `line` (its newline included) as the one JSON object it must be, with a
/// string `type`.
pub fn json_line(line: &str) -> Value {
    let text = line
        .strip_suffix('\n')
        .unwrap_or_else(|| panic!("a line without its newline: {line:?}"));
    assert!(!text.contains('\n'), "{line:?}");
    let value: Value = serde_json::from_str(text)
        .unwrap_or_else(|error| panic!("not one JSON value ({error}): {line:?}"));
    assert!(value["type"].is_string(), "no type: {line:?}");
    value
}

/// Every line of `stdout`, each as [`json_line`]: a stream with anything
/// else on it fails here.
pub fn json_lines(stdout: &[u8]) -> Vec<Value> {
    let text = std::str::from_utf8(stdout).expect("UTF-8 stdout");
    text.split_inclusive('\n').map(json_line).collect()
}

/// Each value's `type`.
pub fn kinds(values: &[Value]) -> Vec<&str> {
    values
        .iter()
        .map(|value| value["type"].as_str().unwrap())
        .collect()
}

/// A child process killed and reaped when dropped, unless it has exited.
pub struct KillOnDrop(pub std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

impl KillOnDrop {
    /// The exit status, within the suite's bound.
    pub async fn exited(&mut self) -> std::process::ExitStatus {
        super::bounded(async {
            loop {
                if let Some(status) = self.0.try_wait().unwrap() {
                    return status;
                }
                // Process-exit polling only.
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
    }

    /// All of stderr, once the process has exited.
    pub fn stderr(&mut self) -> String {
        let mut text = String::new();
        if let Some(mut pipe) = self.0.stderr.take() {
            pipe.read_to_string(&mut text).unwrap();
        }
        text
    }
}

/// A child's stdout read a line at a time as it arrives, by a thread of
/// its own, so a test can wait for each line with a bound while the
/// command runs. The thread ends when stdout does.
pub struct Lines(mpsc::UnboundedReceiver<String>);

impl Lines {
    pub fn read(stdout: impl Read + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => return,
                    Ok(_) if sender.send(line).is_err() => return,
                    Ok(_) => {}
                }
            }
        });
        Self(receiver)
    }

    /// The next line as [`json_line`], within the suite's bound.
    pub async fn next(&mut self) -> Value {
        self.next_or_end().await.expect("stdout ended")
    }

    /// The next line as [`json_line`], or `None` at the end of stdout,
    /// within the suite's bound.
    pub async fn next_or_end(&mut self) -> Option<Value> {
        let line = super::bounded(self.0.recv()).await;
        line.map(|line| json_line(&line))
    }

    /// The lines left, to the end of stdout, within the suite's bound.
    pub async fn rest(&mut self) -> Vec<Value> {
        let mut values = Vec::new();
        while let Some(line) = super::bounded(self.0.recv()).await {
            values.push(json_line(&line));
        }
        values
    }
}
