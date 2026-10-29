//! Observing a guide child's retirement. A sibling of `guide_process.rs`,
//! which the test includes as `guide`.

use super::guide::{Process, Retirement};

impl Process {
    /// Follow the child's retirement, which outlives this owner.
    pub fn retirement(&self) -> Retirement {
        self.retired.clone()
    }
}
