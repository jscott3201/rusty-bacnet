use super::*;
use bacnet_objects::traits::BACnetObject;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Barrier, Semaphore};

struct CountDrop(Arc<AtomicUsize>);
impl Drop for CountDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn failure_reporter() -> bacnet_objects::audit::AuditReporterObject {
    use bacnet_types::{
        bitstring::AuditOperationFlags,
        enums::{AuditLevel, AuditOperation},
    };
    let mut reporter = bacnet_objects::audit::AuditReporterObject::new(1, "AR").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::AUDITING_FAILURE);
    reporter.set_auditable_operations(flags).unwrap();
    reporter.set_issue_confirmed_notifications(true).unwrap();
    reporter
}

fn failure_ticket(
    owner: &NotificationTransactions,
    reporter: &bacnet_objects::audit::AuditReporterObject,
) -> AuditFailureTicket<Arc<ConfirmedRecipientRoute>> {
    let status = reporter.status_internal();
    owner
        .audit_failures
        .get_or_init(|| vec![(Arc::clone(&status), AuditFailureQueue::default())]);
    owner
        .audit_failure_queue(&reporter.status_internal())
        .unwrap()
        .observe(AuditFailureContext {
            device: ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, 10).unwrap(),
            epoch: status.auditing_failure_epoch().unwrap(),
            status,
            confirmed: reporter.confirmed_internal(),
            peer: canonical_direct_peer(&[1]),
            route: Arc::new(ConfirmedRecipientRoute {
                canonical_peer: canonical_direct_peer(&[1]),
                local_target: Some(bacnet_types::MacAddr::from_slice(&[1])),
                remote: None,
                freshness: None,
            }),
            max_apdu: 1476,
        })
        .unwrap()
}

fn record_failure(
    owner: &NotificationTransactions,
    reporter: &bacnet_objects::audit::AuditReporterObject,
    count: u64,
    sequence: u16,
) -> Option<audit_failure_queue::AuditFailureWorker<Arc<ConfirmedRecipientRoute>>> {
    let ticket = failure_ticket(owner, reporter);
    owner
        .audit_failure_queue(&reporter.status_internal())
        .unwrap()
        .record_drop(
            owner,
            ticket,
            BACnetTimeStamp::SequenceNumber(sequence),
            count,
        )
}

#[derive(Default)]
struct CountWake(AtomicUsize);
impl std::task::Wake for CountWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn auditing_failure_wait_is_passive_saturates_and_wakes_on_exact_release() {
    let owner = NotificationTransactions::new();
    let reporter = failure_reporter();
    let mut held: Vec<_> = (0..256).map(|_| reserve(&owner)).collect();
    let mut worker = record_failure(&owner, &reporter, u64::MAX - 1, 65535).unwrap();
    assert!(record_failure(&owner, &reporter, 5, 0).is_none());
    let wake = Arc::new(CountWake::default());
    let waker = std::task::Waker::from(wake.clone());
    let mut cx = std::task::Context::from_waker(&waker);
    {
        let mut next = std::pin::pin!(worker.next());
        assert!(next.as_mut().poll(&mut cx).is_pending());
        assert_eq!(wake.0.load(Ordering::SeqCst), 0, "no self wake/poll loop");
        assert_eq!(owner.audit_resources(), (true, u64::MAX, 63));
        assert!(record_failure(&owner, &reporter, 1, 1).is_none());
        assert_eq!(
            wake.0.load(Ordering::SeqCst),
            0,
            "same-batch drops do not spin the waiter"
        );
        drop(held.pop());
        assert_eq!(wake.0.load(Ordering::SeqCst), 1);
        let Poll::Ready(Some((batch, permit, reserved))) = next.as_mut().poll(&mut cx) else {
            panic!("release must make pending summary admissible")
        };
        assert_eq!(batch.count, u64::MAX);
        assert_eq!(batch.earliest, BACnetTimeStamp::SequenceNumber(65535));
        assert_eq!(
            batch.notification().current_value,
            Some(vec![0x25, 8, 255, 255, 255, 255, 255, 255, 255, 255])
        );
        drop((permit, reserved));
    }
    assert!(worker.next().await.is_none());
    // An old completed guard must not clear a newly registered owner.
    let new_owner = record_failure(&owner, &reporter, 1, 2).unwrap();
    drop(worker);
    assert_eq!(owner.audit_resources(), (true, 1, 64));
    drop(new_owner);
    drop(held);
    assert_eq!(owner.audit_resources(), (false, 0, 64));
    assert_eq!(owner.active_count(), 0);
}

#[tokio::test]
async fn auditing_failure_disabled_pending_batch_cannot_reappear_after_enablement() {
    let owner = NotificationTransactions::new();
    let mut reporter = failure_reporter();
    let permits: Vec<_> = (0..64).map(|_| owner.try_admit_audit().unwrap()).collect();
    let mut worker = record_failure(&owner, &reporter, 3, 0).unwrap();
    let wake = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(wake);
    {
        let mut next = std::pin::pin!(worker.next());
        assert!(next.as_mut().poll(&mut cx).is_pending());
        reporter
            .set_audit_level(bacnet_types::enums::AuditLevel::NONE)
            .unwrap();
        reporter
            .set_audit_level(bacnet_types::enums::AuditLevel::AUDIT_ALL)
            .unwrap();
        drop(permits);
        assert!(matches!(next.as_mut().poll(&mut cx), Poll::Ready(None)));
    }
    assert_eq!(owner.audit_resources(), (false, 0, 64));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auditing_failure_concurrent_drops_register_exactly_one_owner() {
    let owner = NotificationTransactions::new();
    let reporter = Arc::new(failure_reporter());
    let barrier = Arc::new(Barrier::new(33));
    let mut producers = JoinSet::new();
    for _ in 0..32 {
        let (owner, reporter, barrier) = (owner.clone(), reporter.clone(), barrier.clone());
        producers.spawn(async move {
            barrier.wait().await;
            record_failure(&owner, &reporter, 1, 0)
        });
    }
    barrier.wait().await;
    let mut workers = vec![];
    while let Some(result) = producers.join_next().await {
        workers.extend(result.unwrap());
    }
    assert_eq!(workers.len(), 1);
    assert_eq!(owner.audit_resources(), (true, 32, 64));
    drop(workers);
    assert_eq!(owner.audit_resources(), (false, 0, 64));
}

fn reserve(owner: &NotificationTransactions) -> NotificationOperation {
    owner
        .reserve(
            canonical_direct_peer(&[1]),
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        )
        .unwrap()
        .0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notification_worker_spawn_close_race_releases_all_resources() {
    let owner = NotificationTransactions::new();
    let count = Arc::new(AtomicUsize::new(0));
    let permits = Arc::new(Semaphore::new(64));
    let barrier = Arc::new(Barrier::new(65));
    let mut producers = JoinSet::new();
    for _ in 0..64 {
        let operation = reserve(&owner);
        let permit = permits.clone().try_acquire_owned().unwrap();
        let guard = CountDrop(count.clone());
        let owner = owner.clone();
        let barrier = barrier.clone();
        producers.spawn(async move {
            barrier.wait().await;
            owner.spawn(async move {
                let _resources = (operation, permit, guard);
                std::future::pending::<()>().await;
            });
        });
    }
    barrier.wait().await;
    owner.close();
    while let Some(result) = producers.join_next().await {
        result.unwrap();
    }
    while let Some(result) = owner.join_next().await {
        assert!(result.unwrap_err().is_cancelled());
    }
    assert!(owner.workers_empty());
    assert_eq!(owner.active_count(), 0);
    assert_eq!(permits.available_permits(), 64);
    assert_eq!(count.load(Ordering::SeqCst), 64);
    assert!(matches!(
        owner.reserve(
            canonical_direct_peer(&[1]),
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
        ),
        Err(NotificationReserveError::Closed)
    ));
}

#[tokio::test]
async fn notification_worker_post_close_rejects_outside_owner_lock() {
    struct ReentrantDrop(std::sync::Weak<NotificationTransactions>);
    impl Drop for ReentrantDrop {
        fn drop(&mut self) {
            assert!(self.0.upgrade().unwrap().workers_empty());
        }
    }
    let owner = NotificationTransactions::new();
    let operation = reserve(&owner);
    let reentrant = ReentrantDrop(Arc::downgrade(&owner));
    let count = Arc::new(AtomicUsize::new(0));
    let guard = CountDrop(count.clone());
    owner.close();
    owner.spawn(async move {
        let _resources = (operation, reentrant, guard);
        panic!("a post-close future must never be polled");
    });
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(owner.workers_empty());
    assert_eq!(owner.active_count(), 0);
}

#[tokio::test]
async fn notification_worker_empty_wait_wakes_and_cancelled_join_retains_task() {
    let owner = NotificationTransactions::new();
    // Register an empty-set consumer before an independent producer spawns.
    let (polled, ready) = oneshot::channel();
    let consumer_owner = owner.clone();
    let consumer = tokio::spawn(async move {
        let mut join = std::pin::pin!(consumer_owner.join_next());
        poll_fn(|cx| {
            assert!(join.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        polled.send(()).unwrap();
        join.await.unwrap().unwrap();
    });
    ready.await.unwrap();
    owner.spawn(async {});
    tokio::time::timeout(Duration::from_secs(2), consumer)
        .await
        .unwrap()
        .unwrap();
    assert!(owner.workers_empty());

    let (release, held) = oneshot::channel::<()>();
    owner.spawn(async move {
        let _ = held.await;
    });
    {
        let mut join = std::pin::pin!(owner.join_next());
        poll_fn(|cx| {
            assert!(join.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }
    assert!(!owner.workers_empty());
    release.send(()).unwrap();
    owner.join_next().await.unwrap().unwrap();
    assert!(owner.workers_empty());
}

#[tokio::test]
async fn notification_worker_operation_does_not_retain_owner() {
    let owner = NotificationTransactions::new();
    let weak = Arc::downgrade(&owner);
    let operation = reserve(&owner);
    let core = Arc::downgrade(&operation.transactions);
    let (dropped, observed) = oneshot::channel();
    struct NotifyDrop(Option<oneshot::Sender<()>>);
    impl Drop for NotifyDrop {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    let guard = NotifyDrop(Some(dropped));
    owner.spawn(async move {
        let _resources = (operation, guard);
        std::future::pending::<()>().await;
    });
    drop(owner);
    assert!(
        weak.upgrade().is_none(),
        "worker retained its owning JoinSet"
    );
    observed.await.unwrap();
    assert!(core.upgrade().is_none());
}

#[tokio::test]
async fn auditing_failure_requester_only_release_wakes_without_notification_completion() {
    use bacnet_endpoint_core::coordinator::{LeaseMetadata, ReleaseOutcome, TerminalPolicy};
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let owner = NotificationTransactions::with_coordinator(coordinator.clone());
    let reporter = failure_reporter();
    let mut held: Vec<_> = (0..256)
        .map(|_| {
            coordinator
                .reserve(LeaseMetadata::requester(
                    canonical_direct_peer(&[2]),
                    ConfirmedServiceChoice::READ_PROPERTY,
                    TerminalPolicy::ComplexAck,
                ))
                .unwrap()
        })
        .collect();
    let mut worker = record_failure(&owner, &reporter, 1, 9).unwrap();
    let wake = Arc::new(CountWake::default());
    let waker = std::task::Waker::from(wake.clone());
    let mut cx = std::task::Context::from_waker(&waker);
    {
        let mut next = std::pin::pin!(worker.next());
        assert!(next.as_mut().poll(&mut cx).is_pending());
        let token = held.pop().unwrap();
        assert_eq!(
            coordinator.release(token).unwrap(),
            ReleaseOutcome::Released
        );
        assert_eq!(wake.0.load(Ordering::SeqCst), 1);
        // Duplicate cleanup is not capacity; notification registration must not
        // be woken by stale local completion tokens.
        assert_ne!(
            coordinator.release(token).unwrap(),
            ReleaseOutcome::Released
        );
        assert_eq!(wake.0.load(Ordering::SeqCst), 1);
        let Poll::Ready(Some((batch, permit, reserved))) = next.as_mut().poll(&mut cx) else {
            panic!("requester release must admit summary")
        };
        assert_eq!(batch.count, 1);
        drop((permit, reserved));
    }
    for token in held {
        coordinator.release(token).unwrap();
    }
    assert!(worker.next().await.is_none());
    assert_eq!(coordinator.active_count().unwrap(), 0);
}

#[tokio::test]
async fn auditing_failure_admission_order_survives_reversed_completions_and_context_supersession() {
    let owner = NotificationTransactions::new();
    let mut reporter = failure_reporter();
    let first = failure_ticket(&owner, &reporter);
    let second = failure_ticket(&owner, &reporter);
    let queue = owner
        .audit_failure_queue(&reporter.status_internal())
        .unwrap();
    let mut worker = queue
        .record_drop(&owner, second, BACnetTimeStamp::SequenceNumber(0), 1)
        .unwrap();
    assert!(queue
        .record_drop(
            &owner,
            first.clone(),
            BACnetTimeStamp::SequenceNumber(65535),
            1
        )
        .is_none());
    let (batch, permit, reserved) = worker.next().await.unwrap();
    assert_eq!(batch.count, 2);
    assert_eq!(batch.earliest, BACnetTimeStamp::SequenceNumber(65535));
    drop((permit, reserved));
    // Empty pending state must retain the latest context's identity. Mode ABA
    // must not let a delayed contribution recreate the old pending batch.
    reporter.set_issue_confirmed_notifications(false).unwrap();
    reporter.set_issue_confirmed_notifications(true).unwrap();
    let current = failure_ticket(&owner, &reporter);
    assert!(queue
        .record_drop(&owner, first, BACnetTimeStamp::SequenceNumber(65535), 7)
        .is_none());
    assert_eq!(owner.audit_resources(), (true, 0, 64));
    assert!(queue
        .record_drop(&owner, current, BACnetTimeStamp::SequenceNumber(1), 3)
        .is_none());
    let (batch, permit, reserved) = worker.next().await.unwrap();
    assert_eq!(batch.count, 3);
    assert_eq!(batch.earliest, BACnetTimeStamp::SequenceNumber(1));
    drop((permit, reserved));
    assert!(worker.next().await.is_none());
}

#[tokio::test]
async fn auditing_failure_context_change_wakes_separately_and_never_transfers_counts() {
    let owner = NotificationTransactions::new();
    let held: Vec<_> = (0..256).map(|_| reserve(&owner)).collect();
    let mut reporter = failure_reporter();
    let old = failure_ticket(&owner, &reporter);
    let queue = owner
        .audit_failure_queue(&reporter.status_internal())
        .unwrap();
    let mut worker = queue
        .record_drop(&owner, old.clone(), BACnetTimeStamp::SequenceNumber(4), 7)
        .unwrap();
    let wake = Arc::new(CountWake::default());
    let waker = std::task::Waker::from(wake.clone());
    let mut cx = std::task::Context::from_waker(&waker);
    {
        let mut next = std::pin::pin!(worker.next());
        assert!(next.as_mut().poll(&mut cx).is_pending());
        reporter.set_issue_confirmed_notifications(false).unwrap();
        let new = failure_ticket(&owner, &reporter);
        assert_eq!(
            wake.0.load(Ordering::SeqCst),
            1,
            "context change is its own wake reason"
        );
        assert!(queue
            .record_drop(&owner, new, BACnetTimeStamp::SequenceNumber(5), 2)
            .is_none());
        assert!(queue
            .record_drop(&owner, old, BACnetTimeStamp::SequenceNumber(4), 9)
            .is_none());
        let Poll::Ready(Some((batch, permit, reserved))) = next.as_mut().poll(&mut cx) else {
            panic!("unconfirmed context requires no released invoke ID")
        };
        assert_eq!(batch.count, 2);
        assert_eq!(batch.earliest, BACnetTimeStamp::SequenceNumber(5));
        assert!(reserved.is_none());
        assert_eq!(owner.active_count(), 256);
        drop(permit);
    }
    assert!(worker.next().await.is_none());
    drop(held);
}

#[tokio::test]
async fn auditing_failure_timestamp_choice_changes_do_not_change_admission_order() {
    use bacnet_types::primitives::Time;
    let owner = NotificationTransactions::new();
    let reporter = failure_reporter();
    let first = failure_ticket(&owner, &reporter);
    let second = failure_ticket(&owner, &reporter);
    let clock_time = BACnetTimeStamp::Time(Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    });
    let queue = owner
        .audit_failure_queue(&reporter.status_internal())
        .unwrap();
    let mut worker = queue.record_drop(&owner, second, clock_time, 1).unwrap();
    assert!(queue
        .record_drop(&owner, first, BACnetTimeStamp::SequenceNumber(99), 1)
        .is_none());
    let (batch, permit, reserved) = worker.next().await.unwrap();
    assert_eq!(batch.count, 2);
    assert_eq!(batch.earliest, BACnetTimeStamp::SequenceNumber(99));
    drop((permit, reserved));
    assert!(worker.next().await.is_none());
}

#[tokio::test]
async fn target_reporter_loss_queues_are_isolated_with_one_global_budget_and_per_owner_aba() {
    use bacnet_objects::audit::{AuditReporterObject, TargetAuditAssociation};
    let owner = NotificationTransactions::new();
    let mut first = failure_reporter();
    let mut second = AuditReporterObject::new(2, "Second").unwrap();
    second
        .set_audit_level(bacnet_types::enums::AuditLevel::AUDIT_ALL)
        .unwrap();
    let flags = first.configuration_internal().auditable_operations;
    second.set_auditable_operations(flags).unwrap();
    second.set_issue_confirmed_notifications(true).unwrap();
    owner.install_target_audit(TargetAuditAssociation::new(vec![
        (first.object_identifier(), first.status_internal()),
        (second.object_identifier(), second.status_internal()),
    ]));
    let permits: Vec<_> = (0..64).map(|_| owner.try_admit_audit().unwrap()).collect();
    assert!(owner.try_admit_audit().is_err());
    let stale = failure_ticket(&owner, &first);
    let mut a = record_failure(&owner, &first, 2, 10).unwrap();
    let mut b = record_failure(&owner, &second, 3, 20).unwrap();
    assert!(record_failure(&owner, &first, 5, 30).is_none());
    assert!(record_failure(&owner, &second, 7, 40).is_none());
    assert_eq!(owner.audit_resources(), (true, 17, 0));
    let queue = owner.audit_failure_queue(&first.status_internal()).unwrap();
    for confirmed in [false, true] {
        let reservation = queue
            .prepare_context(
                first.status_internal().next_configuration_epoch().unwrap(),
                true,
            )
            .unwrap();
        first.set_issue_confirmed_notifications(confirmed).unwrap();
        reservation.publish();
    }
    let current = failure_ticket(&owner, &first);
    let queue = owner.audit_failure_queue(&first.status_internal()).unwrap();
    assert!(queue
        .record_drop(&owner, stale, BACnetTimeStamp::SequenceNumber(0), 99)
        .is_none());
    assert!(queue
        .record_drop(&owner, current, BACnetTimeStamp::SequenceNumber(50), 11)
        .is_none());
    assert_eq!(
        owner.audit_resources(),
        (true, 127, 0),
        "first Reporter ABA retains historical losses without changing the second Reporter"
    );
    drop(permits);
    let (a_batch, a_permit, a_reservation) = a.next().await.unwrap();
    let (b_batch, b_permit, b_reservation) = b.next().await.unwrap();
    assert_eq!(
        (a_batch.count, a_batch.earliest),
        (106, BACnetTimeStamp::SequenceNumber(0))
    );
    assert_eq!(
        (b_batch.count, b_batch.earliest),
        (10, BACnetTimeStamp::SequenceNumber(20))
    );
    assert_eq!(owner.active_count(), 2);
    drop((a_permit, a_reservation, b_permit, b_reservation));
    let (current, permit, reserved) = a.next().await.unwrap();
    assert_eq!(
        (current.count, current.earliest.clone()),
        (11, BACnetTimeStamp::SequenceNumber(50))
    );
    drop((current, permit, reserved));
    assert!(a.next().await.is_none());
    assert!(b.next().await.is_none());
    assert_eq!(owner.audit_resources(), (false, 0, 64));
    assert_eq!(owner.active_count(), 0);
}
