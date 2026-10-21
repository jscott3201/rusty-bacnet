//! Controlled command-boundary saturation, coupled to actual UI key/event handling.
//! The held inner receiver models a stalled worker; this is not an end-to-end
//! network or FD-start user flow.
use super::*;
use crate::tui::app::OpState;
use crate::tui::message::OpOutcome;
use crate::tui::tests::{connected_app, press, press_with, screen, worker};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

#[tokio::test]
async fn full_inner_queue_reports_cancel_refusal_after_outer_admission() {
    let (sink, mut events) = EventSink::channel(4);
    let (commands, mut held_receiver) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    let (cancel, _) = watch::channel(false);
    let (release, held) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        held.await.unwrap();
    });
    let active = Active {
        cancel,
        commands,
        task,
    };
    let mut app = connected_app();
    app.generation = 7;
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Tab);
    // Local full-range Who-Is needs one confirmation before sending.
    let mut outgoing = press(&mut app, KeyCode::Enter);
    if outgoing.is_empty() {
        outgoing = press(&mut app, KeyCode::Enter);
    }
    let [request @ Command::WhoIs { op, .. }] = outgoing.as_slice() else {
        panic!("unexpected {outgoing:?}");
    };
    let op = *op;
    for _ in 0..COMMAND_CHANNEL_CAPACITY {
        active.commands.try_send(request.clone()).unwrap();
    }
    assert_eq!(active.commands.capacity(), 0);
    assert!(!active.commands.is_closed());
    let [cancel] = press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL)
        .try_into()
        .unwrap();
    assert_eq!(cancel, Command::Cancel { op });
    assert_eq!(app.op.as_ref().unwrap().state, OpState::Cancelled);
    let (outer, mut outer_rx) = mpsc::channel(1);
    outer
        .try_send(cancel)
        .expect("outer UI queue admitted cancellation");
    forward_operation(Some(&active), outer_rx.recv().await.unwrap(), op, 7, &sink).await;
    let refusal = events.recv().await.unwrap();
    // The held worker queue still contains Who-Is work: cancellation did not enter it.
    for _ in 0..COMMAND_CHANNEL_CAPACITY {
        assert!(matches!(
            held_receiver.try_recv().unwrap(),
            Command::WhoIs { .. }
        ));
    }
    assert!(held_receiver.try_recv().is_err());
    let WorkerEvent::Session { generation, event } = &refusal else {
        panic!("unscoped {refusal:?}");
    };
    assert_eq!(*generation, 7);
    assert!(
        matches!(event.as_ref(), WorkerEvent::OpRejected { op: id, cancel: true, error } if *id == op && error.starts_with("session command not admitted:"))
    );
    worker(
        &mut app,
        WorkerEvent::Session {
            generation: 6,
            event: event.clone(),
        },
    );
    assert_eq!(
        app.op.as_ref().unwrap().state,
        OpState::Cancelled,
        "old sessions cannot correct this cancellation"
    );
    worker(
        &mut app,
        WorkerEvent::Session {
            generation: 7,
            event: Box::new(WorkerEvent::OpRejected {
                op: op + 1,
                cancel: true,
                error: "other operation".into(),
            }),
        },
    );
    assert_eq!(
        app.op.as_ref().unwrap().state,
        OpState::Cancelled,
        "other operation IDs cannot correct this cancellation"
    );
    worker(&mut app, refusal);
    assert_ne!(
        app.op.as_ref().unwrap().state,
        OpState::Cancelled,
        "inner queue refusal must correct optimistic cancellation"
    );
    let frame = screen(&mut app, 120, 40);
    assert!(frame.contains("cancel failed"), "{frame}");
    assert!(app.flash.as_ref().unwrap().text.contains("retry"));
    assert!(screen(&mut app, 80, 24).contains("Ctrl-C to retry; x disconnects."));
    let failed = app.op.as_ref().unwrap().state.clone();
    for event in [
        WorkerEvent::WhoIsSent { op },
        WorkerEvent::OpFinished {
            op,
            outcome: OpOutcome::Completed,
        },
        WorkerEvent::OpFinished {
            op,
            outcome: OpOutcome::Failed("late send failure".into()),
        },
    ] {
        worker(
            &mut app,
            WorkerEvent::Session {
                generation: 7,
                event: Box::new(event),
            },
        );
        assert_eq!(
            app.op.as_ref().unwrap().state,
            failed,
            "ordinary late progress cannot erase cancel refusal"
        );
    }
    // Refusal resets the double-Ctrl-C gesture so immediate retry does not quit.
    let retry = press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(retry, [Command::Cancel { op }]);
    assert!(!app.quit);
    outer.try_send(retry[0].clone()).unwrap();
    forward_operation(Some(&active), outer_rx.recv().await.unwrap(), op, 7, &sink).await;
    assert_eq!(held_receiver.try_recv().unwrap(), Command::Cancel { op });
    assert!(
        events.try_recv().is_err(),
        "admitted retry must not produce a rejection"
    );
    release.send(()).unwrap();
    active.task.await.unwrap();
}

#[test]
fn outer_cancel_refusal_uses_the_same_identity_and_retry_contract() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Enter);
    let commands = press(&mut app, KeyCode::Enter);
    let [Command::WhoIs { op, .. }] = commands.as_slice() else {
        panic!("Who-Is required");
    };
    let op = *op;
    let [cancel] = press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL)
        .try_into()
        .unwrap();
    let link = app.link.clone();
    app.command_rejected(
        Command::Cancel { op: op + 1 },
        link.clone(),
        "unrelated".into(),
    );
    assert_eq!(app.op.as_ref().unwrap().state, OpState::Cancelled);
    app.command_rejected(cancel, link, "outer queue full".into());
    assert_eq!(
        app.op.as_ref().unwrap().state,
        OpState::CancelFailed("outer queue full".into())
    );
    assert_eq!(
        press_with(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL),
        [Command::Cancel { op }]
    );
    assert!(!app.quit);
}

/// A browse read that can't reach a session fails at once under its
/// request id, scoped to the generation, so the browser stops waiting.
#[tokio::test]
async fn a_browse_read_without_a_session_fails_under_its_request() {
    let (sink, mut events) = EventSink::channel(4);
    let read = Command::Browse {
        req: 9,
        device: 100,
        read: crate::tui::point::BrowseRead::Device,
    };
    forward_browse(None, read, 9, 3, &sink).await;
    let WorkerEvent::Session { generation, event } = events.recv().await.unwrap() else {
        panic!("unscoped event");
    };
    assert_eq!(generation, 3);
    assert!(matches!(
        *event,
        WorkerEvent::Browse { req: 9, result: BrowseResult::Failed(ref error) }
            if error == "no active connection"
    ));
}

/// A watch command waits for room in a full session queue rather than
/// being dropped: a lost remove would leave a renewed subscription on the
/// device. One that still finds no room after the wait fails its row.
#[tokio::test(start_paused = true)]
async fn a_watch_command_waits_for_room_in_a_full_session_queue() {
    use crate::tui::watched::{Point, Source, WatchCommand, WatchEvent};
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::ObjectIdentifier;

    let (sink, mut events) = EventSink::channel(4);
    let (commands, mut held) = mpsc::channel(1);
    let (cancel, _) = watch::channel(false);
    let active = Active {
        cancel,
        commands,
        task: tokio::spawn(std::future::pending::<()>()),
    };
    active.commands.try_send(Command::Reconnect).unwrap();
    assert_eq!(active.commands.capacity(), 0, "the session queue is full");

    let remove = WatchCommand::Remove { row: 3 };
    let drain = async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(held.recv().await, Some(Command::Reconnect));
        // A dropped command never arrives: give up rather than hang.
        tokio::time::timeout(Duration::from_secs(30), held.recv())
            .await
            .ok()
            .flatten()
    };
    let ((), forwarded) = tokio::join!(
        forward_watch(Some(&active), remove.clone(), 7, &sink),
        drain
    );
    assert_eq!(forwarded, Some(Command::Watch(remove)));

    // Never drained: the add gives up after the wait and its row fails.
    active.commands.try_send(Command::Reconnect).unwrap();
    let add = WatchCommand::Add {
        row: 4,
        point: Point {
            device: 100,
            network: None,
            object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            property: PropertyIdentifier::PRESENT_VALUE,
        },
    };
    forward_watch(Some(&active), add, 7, &sink).await;
    let WorkerEvent::Session { generation, event } = events.recv().await.unwrap() else {
        panic!("unscoped event");
    };
    assert_eq!(generation, 7);
    assert!(matches!(
        *event,
        WorkerEvent::Watch(WatchEvent::Source {
            row: 4,
            source: Source::Failed(_)
        })
    ));
    active.task.abort();
}
