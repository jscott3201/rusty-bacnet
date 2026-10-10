use super::*;

fn exercise<Q: QueueStorage>(mut queue: Q) {
    assert_eq!(
        queue.enqueue_npdu(1, &[1], NetworkPriority::Normal, 0, None),
        Ok(())
    );
    assert_eq!(
        queue.enqueue_npdu(2, &[2], NetworkPriority::LifeSafety, 0, None),
        Ok(())
    );
    assert_eq!(
        queue.enqueue_npdu(
            QueueKind::BROADCAST_DESTINATION,
            &[3],
            NetworkPriority::Normal,
            0,
            None
        ),
        Ok(())
    );
    let mut scratch = [0; 4];
    let first = queue.dequeue_into(0, &mut scratch).unwrap().unwrap();
    assert_eq!(first.destination, 2);
    let second = queue.dequeue_into(0, &mut scratch).unwrap().unwrap();
    assert_eq!(second.destination, 1);
    let third = queue.dequeue_into(0, &mut scratch).unwrap().unwrap();
    assert_eq!(third.kind, QueueKind::Broadcast);
}

#[test]
fn priority_and_destination_selection() {
    #[cfg(feature = "heapless")]
    exercise(HeaplessQueues::<4, 4>::new());
    #[cfg(feature = "alloc")]
    exercise(AllocQueues::new(4, 4));
}

#[test]
fn exact_expiry_boundary_and_atomic_oversize() {
    #[cfg(feature = "heapless")]
    {
        let mut queue = HeaplessQueues::<1, 1>::new();
        assert_eq!(
            queue.enqueue_npdu(1, &[9], NetworkPriority::Normal, 4, Some(4)),
            Err(QueueRejectReason::Expired)
        );
        assert_eq!(queue.depth(QueueKind::Unicast), 0);
        assert_eq!(
            queue.enqueue_npdu(1, &[1, 2], NetworkPriority::Normal, 0, None),
            Ok(())
        );
        assert_eq!(
            queue.enqueue_npdu(1, &[3], NetworkPriority::Normal, 0, None),
            Err(QueueRejectReason::Full)
        );
        assert_eq!(queue.depth(QueueKind::Unicast), 1);
    }
    #[cfg(feature = "alloc")]
    {
        let mut queue = AllocQueues::new(1, 1);
        assert_eq!(
            queue.enqueue_npdu(1, &[9], NetworkPriority::Normal, 4, Some(4)),
            Err(QueueRejectReason::Expired)
        );
        assert_eq!(queue.depth(QueueKind::Unicast), 0);
        assert_eq!(
            queue.enqueue_npdu(1, &[1, 2], NetworkPriority::Normal, 0, None),
            Ok(())
        );
        assert_eq!(
            queue.enqueue_npdu(1, &[3], NetworkPriority::Normal, 0, None),
            Err(QueueRejectReason::Full)
        );
        assert_eq!(queue.depth(QueueKind::Unicast), 1);
    }
}

#[test]
fn starvation_forces_old_item_after_service_opportunities() {
    #[cfg(feature = "heapless")]
    {
        let mut queue = HeaplessQueues::<4, 0>::with_starvation_limit(1);
        queue
            .enqueue_npdu(1, &[1], NetworkPriority::Normal, 0, None)
            .unwrap();
        queue
            .enqueue_npdu(1, &[2], NetworkPriority::LifeSafety, 0, None)
            .unwrap();
        let mut scratch = [0; 2];
        assert_eq!(queue.dequeue_into(0, &mut scratch).unwrap().unwrap().len, 1);
        assert_eq!(scratch[0], 2);
        assert_eq!(queue.dequeue_into(0, &mut scratch).unwrap().unwrap().len, 1);
        assert_eq!(scratch[0], 1);
    }
    #[cfg(feature = "alloc")]
    {
        let mut queue = AllocQueues::with_starvation_limit(4, 0, 1);
        queue
            .enqueue_npdu(1, &[1], NetworkPriority::Normal, 0, None)
            .unwrap();
        queue
            .enqueue_npdu(1, &[2], NetworkPriority::LifeSafety, 0, None)
            .unwrap();
        let mut scratch = [0; 2];
        assert_eq!(queue.dequeue_into(0, &mut scratch).unwrap().unwrap().len, 1);
        assert_eq!(scratch[0], 2);
        assert_eq!(queue.dequeue_into(0, &mut scratch).unwrap().unwrap().len, 1);
        assert_eq!(scratch[0], 1);
    }
}

#[test]
fn kind_specific_selection_preserves_independent_capacity() {
    #[cfg(feature = "heapless")]
    {
        let mut queue = HeaplessQueues::<2, 1>::new();
        queue
            .enqueue_npdu(1, &[1], NetworkPriority::Normal, 0, None)
            .unwrap();
        queue
            .enqueue_npdu(0xff, &[2], NetworkPriority::Normal, 0, None)
            .unwrap();
        assert_eq!(
            queue.peek_kind(QueueKind::Broadcast, 0).unwrap().kind,
            QueueKind::Broadcast
        );
        assert_eq!(
            queue.peek_kind(QueueKind::Unicast, 0).unwrap().kind,
            QueueKind::Unicast
        );
    }
    #[cfg(feature = "alloc")]
    {
        let mut queue = AllocQueues::new(2, 1);
        queue
            .enqueue_npdu(1, &[1], NetworkPriority::Normal, 0, None)
            .unwrap();
        queue
            .enqueue_npdu(0xff, &[2], NetworkPriority::Normal, 0, None)
            .unwrap();
        assert_eq!(
            queue.peek_kind(QueueKind::Broadcast, 0).unwrap().kind,
            QueueKind::Broadcast
        );
        assert_eq!(
            queue.peek_kind(QueueKind::Unicast, 0).unwrap().kind,
            QueueKind::Unicast
        );
    }
}
