//! The shared save writer (#1270).

use super::*;
use std::sync::{Mutex, mpsc};
use std::task::Wake;
use std::thread::ThreadId;
use std::time::{Duration, Instant};

/// A storage stand-in whose saves block until the test lets each one go.
struct Gate {
    started: Mutex<mpsc::Sender<u32>>,
    release: Mutex<mpsc::Receiver<Result<(), Error>>>,
    saved: Mutex<Vec<u32>>,
    threads: Mutex<Vec<ThreadId>>,
}

struct GateHandle {
    started: mpsc::Receiver<u32>,
    release: mpsc::Sender<Result<(), Error>>,
}

fn gate() -> (Arc<Gate>, GateHandle) {
    let (started_tx, started) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    (
        Arc::new(Gate {
            started: Mutex::new(started_tx),
            release: Mutex::new(release_rx),
            saved: Mutex::default(),
            threads: Mutex::default(),
        }),
        GateHandle { started, release },
    )
}

fn writer(gate: &Arc<Gate>) -> SaveWriter<u32> {
    let saving = Arc::clone(gate);
    SaveWriter::new(
        "test-writer".into(),
        move |snapshot: &u32| {
            saving
                .threads
                .lock()
                .unwrap()
                .push(std::thread::current().id());
            saving.started.lock().unwrap().send(*snapshot).unwrap();
            let result = saving.release.lock().unwrap().recv().unwrap();
            if result.is_ok() {
                saving.saved.lock().unwrap().push(*snapshot);
            }
            result
        },
        |_, _| {},
    )
}

const WAIT: Duration = Duration::from_secs(10);

impl SaveWait {
    /// [`block`](Self::block) for at most `timeout` for each event, and say
    /// whether the wait is over.
    fn block_for(&self, timeout: Duration) -> bool {
        let state = self.event.state.lock();
        let (state, _) = self
            .event
            .happened
            .wait_timeout_while(state, timeout, |state| !state.done)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let done = state.done;
        drop(state);
        done && self
            .then
            .as_ref()
            .is_none_or(|then| then.block_for(timeout))
    }
}

#[test]
fn saves_run_in_order_on_the_writer_thread() {
    let (gate, handle) = gate();
    let mut writer = writer(&gate);
    let first = writer.submit(1);
    let second = writer.submit(2);
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 1);
    // The caller is free while the first save runs.
    assert!(!first.is_done());
    handle.release.send(Ok(())).unwrap();
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 2);
    handle
        .release
        .send(Err(Error::Encoding("disk full".into())))
        .unwrap();
    assert!(first.take_outcome().is_ok());
    assert!(second.take_outcome().is_err());
    assert_eq!(*gate.saved.lock().unwrap(), [1]);
    assert_ne!(gate.threads.lock().unwrap()[0], std::thread::current().id());
    // The result went to its one taker.
    assert_eq!(second.succeeded(), Some(false));
    assert!(second.take_outcome().is_err());
}

#[test]
fn a_burst_of_unwaited_saves_coalesces_to_the_latest() {
    let (gate, handle) = gate();
    let mut writer = writer(&gate);
    writer.submit_coalescing(1);
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 1);
    // While the first runs, a burst queues; each replaces the one before.
    for snapshot in 2..=9 {
        writer.submit_coalescing(snapshot);
    }
    handle.release.send(Ok(())).unwrap();
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 9);
    handle.release.send(Ok(())).unwrap();
    writer.wait_idle();
    assert_eq!(*gate.saved.lock().unwrap(), [1, 9]);
}

#[test]
fn a_waited_save_is_never_replaced_and_keeps_its_place() {
    let (gate, handle) = gate();
    let mut writer = writer(&gate);
    writer.submit_coalescing(1);
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 1);
    writer.submit_coalescing(2);
    let waited = writer.submit(3);
    writer.submit_coalescing(4);
    for _ in 0..4 {
        handle.release.send(Ok(())).unwrap();
    }
    writer.wait_idle();
    assert!(waited.take_outcome().is_ok());
    assert_eq!(*gate.saved.lock().unwrap(), [1, 2, 3, 4]);
}

#[test]
fn dropping_the_writer_waits_for_queued_saves() {
    let (gate, handle) = gate();
    let mut writer = writer(&gate);
    writer.submit_coalescing(1);
    writer.submit(2);
    for _ in 0..2 {
        handle.release.send(Ok(())).unwrap();
    }
    drop(writer);
    assert_eq!(*gate.saved.lock().unwrap(), [1, 2]);
}

#[test]
fn a_panicking_save_fails_its_ticket_and_the_writer_goes_on() {
    let mut writer = SaveWriter::new(
        "test-writer".into(),
        |snapshot: &u32| {
            assert_ne!(*snapshot, 1, "storage bug");
            Ok(())
        },
        |_, _| {},
    );
    let failed = writer.submit(1);
    let next = writer.submit(2);
    assert!(failed.take_outcome().is_err());
    assert!(next.take_outcome().is_ok());
}

#[test]
fn the_done_hook_sees_every_result() {
    let results = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&results);
    let mut writer = SaveWriter::new(
        "test-writer".into(),
        |snapshot: &u32| {
            if snapshot.is_multiple_of(2) {
                Ok(())
            } else {
                Err(Error::Encoding("odd".into()))
            }
        },
        move |snapshot, result| seen.lock().unwrap().push((*snapshot, result.is_ok())),
    );
    writer.submit(1);
    writer.submit_coalescing(2);
    writer.wait_idle();
    assert_eq!(*results.lock().unwrap(), [(1, false), (2, true)]);
}

struct Flag(std::sync::atomic::AtomicBool);

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn a_save_wait_wakes_its_task_when_the_save_runs() {
    let (gate, handle) = gate();
    let mut writer = writer(&gate);
    let ticket = writer.submit(1);
    let mut wait = ticket.wait();
    let flag = Arc::new(Flag(Default::default()));
    let waker = Waker::from(Arc::clone(&flag));
    let mut cx = Context::from_waker(&waker);
    assert!(Pin::new(&mut wait).poll(&mut cx).is_pending());
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 1);
    handle.release.send(Ok(())).unwrap();
    writer.wait_idle();
    assert!(flag.0.load(std::sync::atomic::Ordering::SeqCst));
    assert!(Pin::new(&mut wait).poll(&mut cx).is_ready());
    assert!(wait.is_ready());
}

/// A joined wait ends only once every wait in it has, in either order, and
/// wakes its task for the later one (#1526).
#[test]
fn a_joined_wait_ends_with_the_last_of_its_waits() {
    for first_set in [0, 2] {
        let events: Vec<Arc<Event>> = (0..3).map(|_| Arc::default()).collect();
        let waits = events.iter().map(|event| SaveWait::new(Arc::clone(event)));
        let mut joined = waits.reduce(SaveWait::and).unwrap();
        let flag = Arc::new(Flag(Default::default()));
        let waker = Waker::from(Arc::clone(&flag));
        let mut cx = Context::from_waker(&waker);
        let mut order: Vec<usize> = (0..3).collect();
        order.rotate_left(first_set);
        for (count, &index) in order.iter().enumerate() {
            assert!(!joined.is_ready());
            assert!(Pin::new(&mut joined).poll(&mut cx).is_pending());
            flag.0.store(false, std::sync::atomic::Ordering::SeqCst);
            events[index].set();
            if count + 1 < order.len() {
                continue;
            }
            assert!(flag.0.load(std::sync::atomic::Ordering::SeqCst));
        }
        assert!(joined.is_ready());
        assert!(Pin::new(&mut joined).poll(&mut cx).is_ready());
        joined.block();
    }
}

#[test]
fn a_directory_sync_follows_a_rename() {
    let dir = std::env::temp_dir().join(format!("rusty-bacnet-durable-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state");
    std::fs::write(dir.join("state.tmp"), b"x").unwrap();
    std::fs::rename(dir.join("state.tmp"), &path).unwrap();
    sync_parent_dir(&path);
    // A bare file name syncs the working directory, and a directory that is
    // gone is logged: the save it follows has landed either way.
    sync_parent_dir(Path::new("state"));
    let _ = std::fs::remove_dir_all(&dir);
    sync_parent_dir(&path);
}

#[cfg(unix)]
#[test]
fn a_filesystem_that_cannot_sync_a_directory_is_passed_over() {
    let error = std::io::Error::from_raw_os_error;
    for code in [libc::EINVAL, libc::ENOTSUP, libc::EOPNOTSUPP, libc::EBADF] {
        assert!(directory_sync_unsupported(&error(code)), "errno {code}");
    }
    // Other failures are real, and are logged as such.
    for code in [libc::EIO, libc::ENOSPC, libc::EACCES] {
        assert!(!directory_sync_unsupported(&error(code)), "errno {code}");
    }
    assert!(!directory_sync_unsupported(&std::io::Error::other(
        "no errno"
    )));
}

#[test]
fn a_staged_lifetime_counts_from_the_end_of_the_save() {
    let (gate, handle) = gate();
    let mut writer = writer(&gate);
    // Dropped before the writer, so a failed assertion lets a held save go
    // instead of leaving the writer's drop waiting for it.
    let handle = handle;
    let submitted = Instant::now();
    let ticket = writer.submit(1);
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 1);
    // A save still running never outlives the lifetime.
    assert!(!ticket.outlived_at(submitted + STAGED_WRITE_LIFETIME * 10));
    // The save takes longer than the lifetime.
    std::thread::sleep(STAGED_WRITE_LIFETIME + Duration::from_millis(100));
    handle.release.send(Ok(())).unwrap();
    writer.wait_idle();
    let after = Instant::now();
    // Counted from submission the lifetime would be over; counted from the
    // end of the save it has only begun.
    assert!(!ticket.outlived_at(submitted + STAGED_WRITE_LIFETIME + Duration::from_millis(50)));
    assert!(ticket.outlived_at(after + STAGED_WRITE_LIFETIME));
}

#[test]
fn a_ticket_recognizes_only_its_own_wait() {
    let (gate, handle) = gate();
    let mut writer = writer(&gate);
    let handle = handle;
    let first = writer.submit(1);
    let second = writer.submit(2);
    for _ in 0..2 {
        handle.started.recv_timeout(WAIT).unwrap();
        handle.release.send(Ok(())).unwrap();
    }
    writer.wait_idle();
    assert!(first.issued(&first.wait()));
    assert!(!first.issued(&second.wait()));
}

#[test]
fn crc32_matches_the_check_value_and_the_bitwise_form() {
    // The CRC-32 catalogue's check value, over the ASCII digits 1 to 9.
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    assert_eq!(crc32(&[]), 0);
    let bitwise = |data: &[u8]| {
        let mut crc = !0u32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        !crc
    };
    let data: Vec<u8> = (0..=255u8).chain((0..=255u8).rev()).collect();
    for end in 0..data.len() {
        assert_eq!(crc32(&data[..end]), bitwise(&data[..end]));
    }
}

#[test]
fn a_closing_writer_without_a_thread_saves_on_the_callers_thread() {
    let saved = Arc::new(Mutex::new(Vec::new()));
    let saving = Arc::clone(&saved);
    let mut writer = SaveWriter::new(
        "test-writer".into(),
        move |snapshot: &u32| {
            saving
                .lock()
                .unwrap()
                .push((*snapshot, std::thread::current().id()));
            Ok(())
        },
        |_, _| {},
    );
    writer.closing();
    writer.submit_coalescing(1);
    // The save already ran, here.
    assert_eq!(*saved.lock().unwrap(), [(1, std::thread::current().id())]);
    assert!(writer.idle().is_ready());
}

#[test]
fn a_closing_writer_with_a_thread_keeps_saving_on_it() {
    let (gate, handle) = gate();
    let mut writer = writer(&gate);
    writer.submit_coalescing(1);
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 1);
    writer.closing();
    writer.submit_coalescing(2);
    for _ in 0..2 {
        handle.release.send(Ok(())).unwrap();
    }
    writer.wait_idle();
    assert_eq!(*gate.saved.lock().unwrap(), [1, 2]);
    let caller = std::thread::current().id();
    assert!(gate.threads.lock().unwrap().iter().all(|id| *id != caller));
}

/// A thread with nothing left to save ends (none waits in tests), and the
/// next save starts another, in order after the first (#1956).
#[test]
fn an_idle_thread_ends_and_the_next_save_starts_another() {
    let (gate, handle) = gate();
    let mut writer = writer(&gate);
    let first = writer.submit(1);
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 1);
    handle.release.send(Ok(())).unwrap();
    assert!(first.take_outcome().is_ok());
    assert!(writer.thread_ended(), "the idle thread still runs");
    let second = writer.submit(2);
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 2);
    handle.release.send(Ok(())).unwrap();
    assert!(second.take_outcome().is_ok());
    assert_eq!(writer.threads_started(), 2);
    assert_eq!(*gate.saved.lock().unwrap(), [1, 2]);
    let threads = gate.threads.lock().unwrap().clone();
    assert_ne!(
        threads[0], threads[1],
        "a second thread made the second save"
    );
}

/// Within its idle period a thread waits for the next save and makes it:
/// two saves, one thread, still running until the writer goes.
#[test]
fn a_thread_waits_out_its_idle_period_for_the_next_save() {
    let saved = Arc::new(Mutex::new(Vec::new()));
    let saving = Arc::clone(&saved);
    let mut writer = SaveWriter::idling_for(Duration::from_secs(3600), move |n: &u32| {
        saving.lock().unwrap().push(*n);
        Ok(())
    });
    assert!(writer.submit(1).take_outcome().is_ok());
    assert!(writer.submit(2).take_outcome().is_ok());
    assert_eq!(writer.threads_started(), 1);
    assert!(writer.thread_running());
    // The drop ends the waiting thread at once and waits for it.
    drop(writer);
    assert_eq!(*saved.lock().unwrap(), [1, 2]);
}

/// Saves queued as the thread ends land in order, each once: whichever of
/// the end and the queueing takes the queue's lock first, the save finds a
/// thread to wake or starts one. Each round lets the thread end, or race
/// it.
#[test]
fn saves_queued_as_the_thread_ends_are_all_made_in_order() {
    let saved = Arc::new(Mutex::new(Vec::new()));
    let saving = Arc::clone(&saved);
    let mut writer = SaveWriter::idling_for(Duration::ZERO, move |n: &u32| {
        saving.lock().unwrap().push(*n);
        Ok(())
    });
    let mut tickets = Vec::new();
    for n in 0..400 {
        if n % 2 == 0 {
            writer.submit_coalescing(n);
        } else {
            tickets.push(writer.submit(n));
        }
        // A stranded save fails here with a message, not at the run's
        // timeout.
        if n % 7 == 0 {
            assert!(
                writer.idle().block_for(WAIT),
                "a save before {n} was stranded"
            );
        }
    }
    assert!(writer.idle().block_for(WAIT), "a save was stranded");
    drop(writer);
    assert!(
        tickets
            .iter()
            .all(|ticket| ticket.succeeded() == Some(true))
    );
    let saved = saved.lock().unwrap().clone();
    // Coalescing may skip an unwaited save, never reorder or repeat one.
    assert!(saved.windows(2).all(|pair| pair[0] < pair[1]), "{saved:?}");
    assert_eq!(saved.last(), Some(&399));
    for ticketed in (1..400).step_by(2) {
        assert!(saved.contains(&ticketed), "save {ticketed} was lost");
    }
}

/// A closing writer whose thread ended saves on the caller's thread, as one
/// that never started a thread does.
#[test]
fn a_closing_writer_whose_thread_ended_saves_on_the_callers_thread() {
    let saved = Arc::new(Mutex::new(Vec::new()));
    let saving = Arc::clone(&saved);
    let mut writer = SaveWriter::idling_for(Duration::ZERO, move |n: &u32| {
        saving
            .lock()
            .unwrap()
            .push((*n, std::thread::current().id()));
        Ok(())
    });
    assert!(writer.submit(1).take_outcome().is_ok());
    assert!(writer.thread_ended());
    writer.closing();
    writer.submit_coalescing(2);
    let caller = std::thread::current().id();
    let saved = saved.lock().unwrap().clone();
    assert_ne!(saved[0].1, caller);
    assert_eq!(saved[1], (2, caller));
    assert_eq!(writer.threads_started(), 1);
}

/// With no handle on its thread, dropping a writer mid-save still returns
/// only once every queued save has run (#1956).
#[test]
fn dropping_a_writer_mid_save_returns_once_its_saves_have_run() {
    let (gate, handle) = gate();
    let mut writer = writer(&gate);
    writer.submit(1);
    writer.submit(2);
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 1);
    let (dropped_tx, dropped) = mpsc::channel();
    let dropper = std::thread::spawn(move || {
        drop(writer);
        dropped_tx.send(()).unwrap();
    });
    handle.release.send(Ok(())).unwrap();
    assert_eq!(handle.started.recv_timeout(WAIT).unwrap(), 2);
    // Save 2 is still running, so the drop can't have returned.
    assert!(
        dropped.try_recv().is_err(),
        "the drop returned with a save still running"
    );
    handle.release.send(Ok(())).unwrap();
    dropped.recv_timeout(WAIT).expect("the drop returned");
    dropper.join().unwrap();
    assert_eq!(*gate.saved.lock().unwrap(), [1, 2]);
}
