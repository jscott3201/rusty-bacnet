//! A stalled reconnect dial as an event (#1839): the step the dial reached
//! rides on `ConnectFailed`. Needs `sc-tls`, which reports dial steps.

use super::*;

/// A reconnect dial through the connector that stalls past the connect
/// timeout reads as `ConnectFailed` with a dial timeout naming the step the
/// dial reached (#1839).
#[tokio::test(start_paused = true)]
async fn a_stalled_redial_reports_the_step_it_reached() {
    let reconnect = ScReconnectConfig {
        initial_delay_ms: 25,
        max_delay_ms: 25,
        max_retries: 1,
    };
    let stalls = || -> Dial {
        Box::pin(async {
            super::super::super::enter_dial_stage(ScDialStage::TcpConnect);
            super::super::super::enter_dial_stage(ScDialStage::Tls);
            std::future::pending().await
        })
    };
    let (mut transport, _rx, hub, mut events) = started(reconnect, stalls).await;
    hub_closes(hub, &mut events).await;
    assert!(matches!(
        next(&mut events).await,
        ScEvent::ReconnectAttempt { .. }
    ));
    assert_eq!(
        next(&mut events).await,
        ScEvent::ConnectFailed {
            hub: ScActiveHub::Primary,
            cause: ScDisconnectCause::DialTimeout {
                stage: Some(ScDialStage::Tls),
            },
        }
    );
    transport.stop().await.unwrap();
}
