use super::request_tasks_tests::{fixture, HeldTransport};
use super::*;
use std::future::{pending, poll_fn, Future};
use std::task::Poll;
use tokio::sync::oneshot;

struct Released(Option<oneshot::Sender<()>>);

impl Drop for Released {
    fn drop(&mut self) {
        let _ = self.0.take().unwrap().send(());
    }
}

fn slots(server: &mut BACnetServer<HeldTransport>) -> [&mut Option<JoinHandle<()>>; 7] {
    [
        &mut server.fault_detection_task,
        &mut server.event_enrollment_task,
        &mut server.trend_log_task,
        &mut server.schedule_tick_task,
        &mut server.intrinsic_reporting_task,
        &mut server.binary_lighting_operation_task,
        &mut server.cov_purge_task,
    ]
}

async fn cancelled_at(position: usize) {
    let (mut server, _ingress, _sends) = fixture().await;
    // Join every original production task before replacing any slot. The test
    // then drives the actual stop seam with no dispatch/DCC join in its way.
    server.stop().await.unwrap();
    let mut released = Vec::new();
    let mut ids = Vec::new();
    for slot in slots(&mut server) {
        assert!(slot.is_none());
        let (tx, rx) = oneshot::channel();
        let guard = Released(Some(tx));
        let task = tokio::spawn(async move {
            let _guard = guard;
            pending::<()>().await;
        });
        ids.push(task.id());
        released.push(rx);
        *slot = Some(task);
    }
    // Make earlier joins ready without consuming their handles. Current and
    // later tasks remain pending; the single-thread runtime cannot process an
    // abort between the synchronous stop poll and the slot assertions below.
    for slot in slots(&mut server).into_iter().take(position) {
        slot.as_ref().unwrap().abort();
    }
    while slots(&mut server)
        .into_iter()
        .take(position)
        .any(|slot| !slot.as_ref().unwrap().is_finished())
    {
        tokio::task::yield_now().await;
    }
    {
        let mut stop = std::pin::pin!(server.stop());
        poll_fn(|cx| {
            assert!(stop.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }
    for (index, slot) in slots(&mut server).into_iter().enumerate() {
        if index < position {
            assert!(slot.is_none(), "completed slot {index} must be cleared");
        } else {
            assert_eq!(
                slot.as_ref().map(JoinHandle::id),
                Some(ids[index]),
                "cancelled stop lost producer slot {index} at join {position}"
            );
            assert!(!slot.as_ref().unwrap().is_finished());
        }
        assert_eq!(
            released[index].try_recv(),
            if index < position {
                Ok(())
            } else {
                Err(oneshot::error::TryRecvError::Empty)
            }
        );
    }
    // Let the requested abort finish without polling its join. Later producers
    // must still be live, proving stop has not eagerly aborted past this slot.
    while !slots(&mut server)[position].as_ref().unwrap().is_finished() {
        tokio::task::yield_now().await;
    }
    for slot in slots(&mut server).into_iter().skip(position + 1) {
        assert!(!slot.as_ref().unwrap().is_finished());
    }
    server.stop().await.unwrap();
    assert!(slots(&mut server).into_iter().all(|slot| slot.is_none()));
    for resource in released.iter_mut().skip(position) {
        assert_eq!(resource.try_recv(), Ok(()));
    }
    server.stop().await.unwrap();
}

macro_rules! cancellation_test {
    ($name:ident, $position:expr) => {
        #[tokio::test(start_paused = true)]
        async fn $name() {
            cancelled_at($position).await;
        }
    };
}

cancellation_test!(producer_shutdown_cancel_fault_detection, 0);
cancellation_test!(producer_shutdown_cancel_event_enrollment, 1);
cancellation_test!(producer_shutdown_cancel_trend_log, 2);
cancellation_test!(producer_shutdown_cancel_schedule_tick, 3);
cancellation_test!(producer_shutdown_cancel_intrinsic_reporting, 4);
cancellation_test!(producer_shutdown_cancel_binary_lighting, 5);
cancellation_test!(producer_shutdown_cancel_cov_purge, 6);

#[tokio::test(start_paused = true)]
async fn producer_shutdown_completed_panicked_and_absent_slots_are_idempotent() {
    let (mut server, _ingress, _sends) = fixture().await;
    server.stop().await.unwrap();
    // Rotate so every slot is exercised with each terminal/absent state.
    for rotation in 0..3 {
        let mut released = Vec::new();
        for (index, slot) in slots(&mut server).into_iter().enumerate() {
            assert!(slot.is_none());
            let state = (index + rotation) % 3;
            if state == 0 {
                continue;
            }
            let (tx, rx) = oneshot::channel();
            let guard = Released(Some(tx));
            *slot = Some(tokio::spawn(async move {
                let _guard = guard;
                assert_ne!(state, 2, "injected producer panic");
            }));
            released.push(rx);
        }
        while slots(&mut server)
            .into_iter()
            .any(|slot| slot.as_ref().is_some_and(|task| !task.is_finished()))
        {
            tokio::task::yield_now().await;
        }
        server.stop().await.unwrap();
        assert!(slots(&mut server).into_iter().all(|slot| slot.is_none()));
        for mut resource in released {
            assert_eq!(resource.try_recv(), Ok(()));
        }
        server.stop().await.unwrap();
    }
}
