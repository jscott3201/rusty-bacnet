//! The server-facing side of a Staging object: the plans its stage changes
//! queue, how they ended, and the NULLs that give up its slots (`held`).

use crate::prelude::*;

use bacnet_types::enums::Reliability;

use crate::schedule::ScheduleWrite;
use crate::traits::StagingSource;

use super::StagingWritePlan;

impl StagingSource for super::StagingObject {
    fn take_staging_write_plan(&mut self) -> Option<StagingWritePlan> {
        if self.present_stage == 0 {
            self.evaluate(false);
        }
        self.pending_plan.take()
    }

    fn staging_generation(&self) -> u64 {
        self.generation
    }

    fn complete_staging_write_plan(&mut self, generation: u64, success: bool) -> bool {
        if generation != self.generation || self.out_of_service {
            return false;
        }
        let reliability = if success {
            Reliability::NO_FAULT_DETECTED
        } else {
            Reliability::UNRELIABLE_OTHER
        };
        if reliability == self.reliability {
            return false;
        }
        self.reliability = reliability;
        true
    }

    fn take_owed_relinquishes(&mut self) -> Vec<ScheduleWrite> {
        // Out of service the targets are left alone (Clause 12.62.11).
        if self.out_of_service {
            return Vec::new();
        }
        self.held.take_owed_writes()
    }

    fn take_relinquish_writes(&mut self) -> Vec<ScheduleWrite> {
        // A plan never taken wrote nothing, but its slots are recorded; the
        // ownership check skips them.
        self.pending_plan = None;
        self.held.take_relinquish_writes()
    }
}
