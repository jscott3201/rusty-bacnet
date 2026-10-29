//! The browser's scheduling under the session's request limits
//! ([`RequestLimits`](bacnet_ops::limits::RequestLimits)).
//!
//! Within the browser, a device runs one job at a time. Name reads for the
//! rows on screen (#1896) are jobs too, the lowest priority: any other read
//! stops one, and it resumes after. They take no slot of their own, so
//! under a cap that watch polling keeps saturated they can wait
//! indefinitely.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::tui::point::{BrowseRead, ReqId};

/// A browse request waiting for, or holding, its device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Job {
    /// Request id.
    pub(crate) req: ReqId,
    /// What it reads.
    pub(crate) read: BrowseRead,
}

/// What a started job does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Task {
    /// A device or object read.
    Read(Job),
    /// Read the names the device's name queue holds, until it is empty.
    Names,
}

/// A job the caller should start now.
#[derive(Debug)]
pub(crate) struct Started {
    /// Its device.
    pub(crate) device: u32,
    /// The job.
    pub(crate) task: Task,
    /// Set to stop the job before its next request.
    pub(crate) cancel: Arc<AtomicBool>,
}

/// What a running job reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Device,
    Object,
    Names,
}

#[derive(Default)]
struct Slot {
    /// The running job: what it reads, and its stop flag.
    running: Option<(Kind, Arc<AtomicBool>)>,
    /// The newest device read waiting.
    device: Option<Job>,
    /// The newest object read waiting.
    object: Option<Job>,
}

/// Which job each device runs next. A device runs one job at a time; a
/// newer request replaces an older waiting one of its kind, and stops a
/// running one it makes pointless: a device read stops anything, an object
/// read stops another object read or a name read. Name reads run when
/// nothing else waits.
#[derive(Default)]
pub(crate) struct Scheduler {
    slots: HashMap<u32, Slot>,
}

impl Scheduler {
    /// Queue `job` for `device`, returning it to start when the device is idle.
    pub(crate) fn submit(&mut self, device: u32, job: Job) -> Option<Started> {
        let slot = self.slots.entry(device).or_default();
        let reads_device = job.read == BrowseRead::Device;
        let Some((running, cancel)) = &slot.running else {
            return Some(Self::start(slot, device, Task::Read(job)));
        };
        if reads_device || *running != Kind::Device {
            cancel.store(true, Ordering::Relaxed);
        }
        if reads_device {
            slot.object = None;
            slot.device = Some(job);
        } else {
            slot.object = Some(job);
        }
        None
    }

    /// `device`'s name queue holds names to read: a name job to start now
    /// when the device is idle. A running job leaves them for later (a
    /// running name job takes them itself).
    pub(crate) fn submit_names(&mut self, device: u32) -> Option<Started> {
        let slot = self.slots.entry(device).or_default();
        if slot.running.is_some() {
            return None;
        }
        Some(Self::start(slot, device, Task::Names))
    }

    /// The running job for `device` ended; the next one to start, if any.
    /// `names` says the device's name queue still holds names, which are
    /// read once no other job waits.
    pub(crate) fn finished(&mut self, device: u32, names: bool) -> Option<Started> {
        let slot = self.slots.get_mut(&device)?;
        slot.running = None;
        match slot.device.take().or_else(|| slot.object.take()) {
            Some(job) => Some(Self::start(slot, device, Task::Read(job))),
            None if names => Some(Self::start(slot, device, Task::Names)),
            None => {
                self.slots.remove(&device);
                None
            }
        }
    }

    /// Stop everything for `device`: drop its waiting jobs and stop the
    /// running one before its next request.
    pub(crate) fn stop(&mut self, device: u32) {
        if let Some(slot) = self.slots.get_mut(&device) {
            slot.device = None;
            slot.object = None;
            if let Some((_, cancel)) = &slot.running {
                cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    fn start(slot: &mut Slot, device: u32, task: Task) -> Started {
        let cancel = Arc::new(AtomicBool::new(false));
        let kind = match &task {
            Task::Read(job) if job.read == BrowseRead::Device => Kind::Device,
            Task::Read(_) => Kind::Object,
            Task::Names => Kind::Names,
        };
        slot.running = Some((kind, Arc::clone(&cancel)));
        Started {
            device,
            task,
            cancel,
        }
    }
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

    use super::*;

    fn object(req: ReqId, instance: u32) -> Job {
        Job {
            req,
            read: BrowseRead::Object(
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
            ),
        }
    }

    fn device(req: ReqId) -> Job {
        Job {
            req,
            read: BrowseRead::Device,
        }
    }

    /// A device never runs two jobs: a second request waits, the newest of
    /// its kind wins, and it starts only when the running one finishes.
    #[test]
    fn one_job_per_device_and_the_newest_waiting_wins() {
        let mut scheduler = Scheduler::default();
        let first = scheduler.submit(7, device(1)).expect("idle device starts");
        assert!(scheduler.submit(7, object(2, 1)).is_none());
        assert!(scheduler.submit(7, object(3, 2)).is_none());
        assert!(
            !first.cancel.load(Ordering::Relaxed),
            "an object read doesn't stop the device read"
        );
        // Another device is independent.
        assert!(scheduler.submit(8, object(4, 1)).is_some());
        let next = scheduler
            .finished(7, false)
            .expect("the waiting object read");
        assert_eq!(next.task, Task::Read(object(3, 2)));
        assert!(scheduler.finished(7, false).is_none());
        assert!(scheduler.submit(7, object(5, 3)).is_some(), "idle again");
    }

    #[test]
    fn a_newer_read_stops_the_running_one_it_replaces() {
        let mut scheduler = Scheduler::default();
        let running = scheduler.submit(7, object(1, 1)).unwrap();
        assert!(scheduler.submit(7, object(2, 2)).is_none());
        assert!(running.cancel.load(Ordering::Relaxed));
        // A device read replaces a waiting object read too.
        assert!(scheduler.submit(7, device(3)).is_none());
        assert_eq!(
            scheduler.finished(7, false).unwrap().task,
            Task::Read(device(3))
        );
        let running = scheduler.finished(7, false);
        assert!(running.is_none(), "the object read was dropped");
    }

    #[test]
    fn stop_drops_waiting_jobs_and_stops_the_running_one() {
        let mut scheduler = Scheduler::default();
        let running = scheduler.submit(7, device(1)).unwrap();
        scheduler.submit(7, object(2, 1));
        scheduler.stop(7);
        assert!(running.cancel.load(Ordering::Relaxed));
        assert!(scheduler.finished(7, false).is_none());
    }

    /// A name job runs only on an idle device, any other read stops it,
    /// and it resumes after them while names are left to read.
    #[test]
    fn name_reads_wait_for_and_give_way_to_every_other_read() {
        let mut scheduler = Scheduler::default();
        let running = scheduler.submit(7, object(1, 1)).unwrap();
        assert!(scheduler.submit_names(7).is_none(), "the device is busy");
        assert!(!running.cancel.load(Ordering::Relaxed));
        let names = scheduler.finished(7, true).expect("names next");
        assert_eq!(names.task, Task::Names);
        assert!(
            scheduler.submit_names(7).is_none(),
            "the running one takes them"
        );
        assert!(scheduler.submit(7, object(2, 2)).is_none());
        assert!(
            names.cancel.load(Ordering::Relaxed),
            "an object read stops it"
        );
        let next = scheduler.finished(7, true).unwrap();
        assert_eq!(next.task, Task::Read(object(2, 2)));
        assert_eq!(scheduler.finished(7, true).unwrap().task, Task::Names);
        assert!(scheduler.finished(7, false).is_none(), "nothing left");
        assert_eq!(scheduler.submit_names(8).unwrap().task, Task::Names);
    }
}
