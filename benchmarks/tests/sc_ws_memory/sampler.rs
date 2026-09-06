use super::*;
use std::io::Write;
use std::sync::{
    atomic::{AtomicBool, AtomicU8, Ordering},
    Arc,
};

pub struct Sampler {
    pub phase: Arc<AtomicU8>,
    stop: Arc<AtomicBool>,
    task: Option<std::thread::JoinHandle<Vec<Value>>>,
}

impl Sampler {
    pub fn start(pid: u32, path: &std::path::Path) -> Self {
        let mut output = std::fs::File::create(path).unwrap();
        let phase = Arc::new(AtomicU8::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let current_phase = phase.clone();
        let should_stop = stop.clone();
        let task = std::thread::spawn(move || {
            let started = Instant::now();
            let mut system = sysinfo::System::new();
            let pid = sysinfo::Pid::from_u32(pid);
            let mut samples = Vec::new();
            while !should_stop.load(Ordering::Acquire) {
                let refresh_start = Instant::now();
                system.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::Some(&[pid]),
                    true,
                    sysinfo::ProcessRefreshKind::nothing().with_memory(),
                );
                let memory = system
                    .process(pid)
                    .map(|p| p.memory())
                    .filter(|rss| *rss > 0);
                let mut sample = json!({"elapsed_us":started.elapsed().as_micros(),"phase":current_phase.load(Ordering::Acquire),"rss_bytes":memory,"refresh_us":refresh_start.elapsed().as_micros()});
                if memory.is_none() {
                    sample["error"] = json!("victim PID missing or zero RSS");
                }
                if memory.is_some_and(|rss| rss > 512 * 1024 * 1024) {
                    sample["error"] = json!("512MiB emergency cap exceeded; victim terminated");
                    if let Some(process) = system.process(pid) {
                        process.kill();
                    }
                }
                writeln!(output, "{sample}").unwrap();
                let failed = sample.get("error").is_some();
                samples.push(sample);
                if failed {
                    break;
                }
                std::thread::sleep(
                    Duration::from_millis(10).saturating_sub(refresh_start.elapsed()),
                );
            }
            samples
        });
        Self {
            phase,
            stop,
            task: Some(task),
        }
    }

    pub fn phase(&self, phase: u8) {
        self.phase.store(phase, Ordering::Release);
    }

    pub fn finish(&mut self) -> Vec<Value> {
        self.stop.store(true, Ordering::Release);
        self.task.take().unwrap().join().unwrap()
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}
