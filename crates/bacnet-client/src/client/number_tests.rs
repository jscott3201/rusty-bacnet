//! Actual client intake and output; no direct calls to the Number parser.
mod support;
use support::*;

/// The client takes the number from the answer to its question and then
/// answers a question sent to it with that number.
#[tokio::test(start_paused = true)]
async fn client_number_learns_and_answers_through_network_intake() {
    let (mut client, inbound, mut outbound, _) = harness(false).await;
    teach(&client, &inbound, &mut outbound, 77, 1).await;
    inject(&inbound, QUERY, false).await;
    let sent = bounded(outbound.recv()).await.unwrap();
    assert!(sent.destination.is_empty());
    assert_eq!(sent.npdu.as_ref(), &[1, 0x80, 0x13, 0, 77, 0]);
    client.stop().await.unwrap();
}

mod answering;
mod asking;
mod claims;
mod folding;
mod lifecycle;
mod local_routes;
mod progress;

/// Controls are taken in order; an unknown number answers nothing; each
/// window of answers is applied by Clause 6.4.20's precedence; and nothing
/// malformed, routed or unicast teaches or is answered.
#[tokio::test(start_paused = true)]
async fn client_number_fifo_unknown_precedence_and_invalid_recovery() {
    let (mut client, inbound, mut outbound, _) = harness(false).await;
    for group in [false, true] {
        inject(&inbound, QUERY, group).await;
    }
    // Unicast: no announcement at all, so no cue either.
    inject(&inbound, &number(999, 1), false).await;
    inject(&inbound, QUERY, true).await;
    for (n, flag, expected) in [
        (77, 0, 77),
        (78, 0, 78),
        (78, 1, 78),
        (79, 0, 78),
        (80, 1, 80),
    ] {
        teach(&client, &inbound, &mut outbound, n, flag).await;
        // Asked of the client alone, so the answer comes at once; one to
        // a broadcast question waits (#2597, `answering`).
        inject(&inbound, QUERY, false).await;
        reply(&mut outbound, expected).await;
    }
    for bytes in [
        number(0, 1),
        number(65535, 1),
        number(999, 2),
        vec![1, 0x80, 0x13, 0, 99],
        vec![1, 0x80, 0x13, 0, 99, 1, 0],
        vec![1, 0x88, 0, 4, 1, 9, 0x13, 0, 99, 1],
        vec![1, 0xa0, 255, 255, 0, 255, 0x13, 0, 99, 1],
    ] {
        inject(&inbound, &bytes, true).await;
        inject(&inbound, QUERY, false).await;
        reply(&mut outbound, 80).await;
    }
    for (i, bytes) in [
        vec![1, 0x80, 0x12, 0],
        vec![1, 0x88, 0, 4, 1, 9, 0x12],
        vec![1, 0xa0, 255, 255, 0, 255, 0x12],
    ]
    .into_iter()
    .enumerate()
    {
        inject(&inbound, &bytes, true).await;
        teach(&client, &inbound, &mut outbound, 81 + i as u16, 1).await;
        inject(&inbound, QUERY, false).await;
        reply(&mut outbound, 81 + i as u16).await;
    }
    client.stop().await.unwrap();
    drop(client);
    assert!(
        bounded(outbound.recv()).await.is_none(),
        "no extra reply after FIFO fence and stop"
    );
}
