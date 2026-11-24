//! A node takes a seated server's handle on the database back (#2766),
//! where a drop leaves its release to a task.
use super::tests::node;
use super::*;

/// Once the seam returns, the caller's handle and the one it returned are
/// the only ones: no task of the server's still holds one, whether the
/// node stopped the server first or not.
#[tokio::test(start_paused = true)]
async fn a_node_takes_back_its_servers_handle_on_the_database() {
    for stopped in [true, false] {
        let (mut node, seat) = node().await;
        let mut server = BACnetServer::generic_builder()
            .seat(seat)
            .build()
            .await
            .unwrap();
        let db = Arc::clone(server.database());
        if stopped {
            server.stop().await.unwrap();
        }
        let back = server.__endpoint_into_database().await;
        assert!(Arc::ptr_eq(&back, &db));
        drop(back);
        assert_eq!(Arc::strong_count(&db), 1, "stopped first: {stopped}");
        node.ingress.stop().await.unwrap();
    }
}

/// The seam's join keeps each task in its slot until it has ended: one
/// cancelled while it waits leaves the rest with the server, whose drop
/// then joins them before it lets go of the database.
#[tokio::test(start_paused = true)]
async fn a_cancelled_retire_leaves_the_drop_its_tasks() {
    let (mut node, seat) = node().await;
    let mut server = BACnetServer::generic_builder()
        .seat(seat)
        .build()
        .await
        .unwrap();
    let db = Arc::clone(server.database());
    {
        let retire = server.retire();
        tokio::pin!(retire);
        // Its first poll aborts every task and waits for the first to end.
        assert!(futures_util::poll!(retire.as_mut()).is_pending());
    }
    assert!(server.dispatch_task.is_some(), "the task it waited for");
    assert!(server.cov_revisit_task.is_some(), "a task it never reached");
    drop(server);
    for _ in 0..16 {
        if Arc::strong_count(&db) == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(Arc::strong_count(&db), 1);
    node.ingress.stop().await.unwrap();
}
