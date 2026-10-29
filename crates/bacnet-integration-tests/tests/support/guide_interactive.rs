//! An interactive guide child: input on stdin, stdout observed as it
//! arrives. A sibling of `guide_process.rs`, which the test includes as
//! `guide`.

use super::guide::Process;
use std::ops::Deref;
use std::path::Path;
use std::process::Output;
use tokio::sync::watch;

/// A [`Process`] whose stdin takes one input and whose stdout can be
/// watched before it exits. Dropping it retires the child as `Process` does.
pub struct Interactive {
    process: Process,
    stdout: watch::Receiver<Vec<u8>>,
}

impl Interactive {
    pub fn start(path: &Path, args: &[&str]) -> Result<Self, String> {
        let (process, stdout) = Process::start(path, args, true)?;
        Ok(Self { process, stdout })
    }

    pub fn stdout(&self) -> watch::Receiver<Vec<u8>> {
        self.stdout.clone()
    }

    pub fn send_input(&mut self, bytes: &[u8]) {
        self.process
            .input
            .take()
            .expect("interactive input is supplied once")
            .send(bytes.to_vec())
            .expect("child input owner remains available");
    }

    pub async fn finish(self) -> Output {
        self.process.finish().await
    }
}

impl Deref for Interactive {
    type Target = Process;

    fn deref(&self) -> &Process {
        &self.process
    }
}
