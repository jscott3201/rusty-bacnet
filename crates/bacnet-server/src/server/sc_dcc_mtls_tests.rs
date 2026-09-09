//! Standalone SC service-path evidence, not certificate-bound DCC principals.
use crate::server::*;
use bacnet_transport::sc::{ScConnectError, ScWebSocketErrorKind};
use bacnet_transport::sc_tls::TlsWebSocket;
use bacnet_types::enums::EnableDisable;
const DISABLE: EnableDisable = EnableDisable::DISABLE;
const DISABLE_INITIATION: EnableDisable = EnableDisable::DISABLE_INITIATION;
const ENABLE: EnableDisable = EnableDisable::ENABLE;

#[path = "sc_dcc_mtls_support.rs"]
mod support;
use support::{bounded, run, Certificates, Outcome, PASSWORD, PEERS};

#[tokio::test]
async fn sc_dcc_mtls_requires_client_and_server_trust() {
    run(async |f| {
        let certs = Certificates::new();
        f.hub(&certs).await;
        for tls in [&certs.missing, &certs.untrusted, &certs.wrong_server_trust] {
            let result = bounded(TlsWebSocket::connect(&f.url, tls.clone())).await;
            // TLS1.3 client authentication rejection may surface during the
            // WebSocket upgrade. A completed error, never a deadline, is required.
            let error = result
                .err()
                .expect("unauthenticated TLS endpoint was admitted");
            assert!(
                matches!(
                    ScConnectError::from_error(&error),
                    Some(ScConnectError::WebSocket {
                        kind: ScWebSocketErrorKind::TlsHandshake
                            | ScWebSocketErrorKind::WebSocketHandshake,
                        ..
                    })
                ),
                "expected TLS/upgrade rejection, not a dial/timeout error: {error:?}"
            );
        }
        f.start(&certs, BACnetServer::sc_builder()).await;
        assert_eq!(
            f.server().dcc_outcome_counters(),
            DccOutcomeCounters::default()
        );
    })
    .await;
}

#[tokio::test]
async fn sc_dcc_mtls_default_denies_even_correct_password() {
    for configured in [false, true] {
        run(async |f| {
            let certs = Certificates::new();
            f.hub(&certs).await;
            let mut builder = BACnetServer::sc_builder(); // Intentionally no policy opt-in.
            if configured {
                builder = builder.dcc_password(PASSWORD);
            }
            f.start(&certs, builder).await;
            for mode in [ENABLE, DISABLE_INITIATION, DISABLE] {
                for password in [None, Some("wrong"), Some(PASSWORD)] {
                    let expected = if configured && password != Some(PASSWORD) {
                        Outcome::Password
                    } else if mode == DISABLE {
                        Outcome::Deprecated
                    } else {
                        Outcome::Policy
                    };
                    f.dcc(0, mode, Some(1), password, None, expected).await;
                    assert_eq!(f.server().comm_state(), 0);
                    assert!(f.server().dcc_timer.lock().await.is_none());
                }
            }
            // A second independently certified hub peer is equally unauthorized.
            f.dcc(
                1,
                DISABLE_INITIATION,
                None,
                Some(PASSWORD),
                None,
                Outcome::Policy,
            )
            .await;
            f.read_property(0).await;
            f.read_property(1).await;
        })
        .await;
    }
}

#[tokio::test]
async fn sc_dcc_mtls_exact_vmac_restriction_and_live_timer() {
    run(async |f| {
        let certs = Certificates::new();
        f.hub(&certs).await;
        let restriction =
            DccSourceRestriction::new(vec![DccSource::Direct(PEERS[0].to_vec())]).unwrap();
        f.start(
            &certs,
            BACnetServer::sc_builder()
                .dcc_policy(DccPolicy::RequirePassword)
                .dcc_password(PASSWORD)
                .dcc_source_restriction(Some(restriction)),
        )
        .await;
        f.dcc(
            0,
            DISABLE_INITIATION,
            Some(1),
            Some(PASSWORD),
            None,
            Outcome::Accepted,
        )
        .await;
        assert_eq!(f.server().comm_state(), 2);
        let timer = f.server().dcc_timer.clone();
        let slot = bounded(timer.lock()).await;
        let task = slot.as_ref().unwrap();
        let id = task.id();
        // Hold the real live timer lock through the exchanges. Denials must
        // neither wait for it nor replace/cancel it, even for ENABLE/zero/longer.
        for duration in [None, Some(0), Some(1), Some(5)] {
            for mode in [ENABLE, DISABLE_INITIATION] {
                f.dcc(1, mode, duration, Some(PASSWORD), None, Outcome::Policy)
                    .await;
                assert_eq!(f.server().comm_state(), 2);
                assert_eq!(slot.as_ref().unwrap().id(), id);
                assert!(!task.is_finished());
            }
        }
        f.dcc(1, ENABLE, None, Some("wrong"), None, Outcome::Password)
            .await;
        f.dcc(1, DISABLE, None, Some(PASSWORD), None, Outcome::Deprecated)
            .await;
        // Routed origin must not be silently treated as this allowed direct VMAC.
        let routed = NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[44]),
        };
        f.dcc(
            0,
            ENABLE,
            None,
            Some(PASSWORD),
            Some(routed),
            Outcome::Policy,
        )
        .await;
        assert_eq!(f.server().comm_state(), 2);
        assert!(!task.is_finished());
        let old = task.abort_handle();
        drop(slot);
        f.read_property(1).await; // DISABLE_INITIATION still executes normal requests.
        f.dcc(0, ENABLE, None, Some(PASSWORD), None, Outcome::Accepted)
            .await;
        assert_eq!(f.server().comm_state(), 0);
        assert!(old.is_finished());
        assert!(timer.lock().await.is_none());
        f.dcc(
            1,
            DISABLE_INITIATION,
            Some(5),
            Some(PASSWORD),
            None,
            Outcome::Policy,
        )
        .await;
        assert_eq!(f.server().comm_state(), 0);
        assert!(timer.lock().await.is_none());
    })
    .await;
}

#[tokio::test]
async fn sc_dcc_mtls_empty_restriction_denies_trusted_peers() {
    run(async |f| {
        let certs = Certificates::new();
        f.hub(&certs).await;
        f.start(
            &certs,
            BACnetServer::sc_builder()
                .dcc_policy(DccPolicy::RequirePassword)
                .dcc_password(PASSWORD)
                .dcc_source_restriction(Some(DccSourceRestriction::new(vec![]).unwrap())),
        )
        .await;
        for peer in 0..2 {
            for mode in [ENABLE, DISABLE_INITIATION] {
                f.dcc(peer, mode, Some(1), Some(PASSWORD), None, Outcome::Policy)
                    .await;
                assert_eq!(f.server().comm_state(), 0);
                assert!(f.server().dcc_timer.lock().await.is_none());
            }
            f.read_property(peer).await;
        }
    })
    .await;
}

#[tokio::test]
async fn sc_dcc_mtls_global_burst_enable_exemption_and_uncharged_failures() {
    run(async |f| {
        let certs = Certificates::new();
        f.hub(&certs).await;
        f.start(
            &certs,
            BACnetServer::sc_builder()
                .dcc_policy(DccPolicy::RequirePassword)
                .dcc_password(PASSWORD)
                .dcc_disable_rate_limit(Some(DccDisableRateLimit::default()))
                .dcc_source_restriction(Some(
                    DccSourceRestriction::new(vec![
                        DccSource::Direct(PEERS[0].to_vec()),
                        DccSource::Direct(PEERS[1].to_vec()),
                        DccSource::Routed {
                            network: 8,
                            address: vec![45],
                        },
                    ])
                    .unwrap(),
                )),
        )
        .await;
        // Keep the actual 3/20s defaults. A short whole-case deadline prevents
        // scheduler delays/refills from turning this into ambiguous rate evidence.
        bounded(async {
            let before = f.server().dcc_outcome_counters();
            let (id, response) = f
                .exchange(
                    0,
                    ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
                    Bytes::new(),
                    None,
                )
                .await;
            let Apdu::Error(error) = response else {
                panic!("expected malformed DCC Error")
            };
            assert_eq!(error.invoke_id, id);
            assert_eq!(
                error.service_choice,
                ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL
            );
            assert_eq!(
                (error.error_class, error.error_code),
                (ErrorClass::SERVICES, ErrorCode::OTHER)
            );
            assert_eq!(
                f.server().dcc_outcome_counters(),
                DccOutcomeCounters {
                    malformed_total: before.malformed_total + 1,
                    ..before
                }
            );
            f.dcc(
                0,
                DISABLE_INITIATION,
                Some(1),
                Some(PASSWORD),
                Some(NpduAddress {
                    network: 9,
                    mac_address: MacAddr::from_slice(&[45]),
                }),
                Outcome::Policy,
            )
            .await;
            for peer in 0..2 {
                f.dcc(
                    peer,
                    DISABLE_INITIATION,
                    None,
                    None,
                    None,
                    Outcome::Password,
                )
                .await;
                f.dcc(
                    peer,
                    DISABLE_INITIATION,
                    None,
                    Some("wrong"),
                    None,
                    Outcome::Password,
                )
                .await;
                f.dcc(
                    peer,
                    DISABLE,
                    None,
                    Some(PASSWORD),
                    None,
                    Outcome::Deprecated,
                )
                .await;
            }
            f.dcc(
                0,
                DISABLE_INITIATION,
                Some(1),
                Some(PASSWORD),
                None,
                Outcome::Accepted,
            )
            .await;
            f.dcc(1, ENABLE, None, Some(PASSWORD), None, Outcome::Accepted)
                .await;
            assert_eq!(f.server().comm_state(), 0);
            f.dcc(
                1,
                DISABLE_INITIATION,
                Some(1),
                Some(PASSWORD),
                None,
                Outcome::Accepted,
            )
            .await;
            f.dcc(
                0,
                DISABLE_INITIATION,
                Some(1),
                Some(PASSWORD),
                None,
                Outcome::Accepted,
            )
            .await;
            let timer = f.server().dcc_timer.clone();
            let slot = timer.lock().await;
            let id = slot.as_ref().unwrap().id();
            let old = slot.as_ref().unwrap().abort_handle();
            for peer in 0..2 {
                for duration in [None, Some(0), Some(5)] {
                    f.dcc(
                        peer,
                        DISABLE_INITIATION,
                        duration,
                        Some(PASSWORD),
                        None,
                        Outcome::Policy,
                    )
                    .await;
                    assert_eq!(f.server().comm_state(), 2);
                    assert_eq!(slot.as_ref().unwrap().id(), id);
                    assert!(!old.is_finished());
                }
            }
            // A routed claimed identity consumes the same depleted server budget.
            f.dcc(
                1,
                DISABLE_INITIATION,
                Some(5),
                Some(PASSWORD),
                Some(NpduAddress {
                    network: 8,
                    mac_address: MacAddr::from_slice(&[45]),
                }),
                Outcome::Policy,
            )
            .await;
            drop(slot);
            for peer in 0..2 {
                f.dcc(peer, ENABLE, None, Some(PASSWORD), None, Outcome::Accepted)
                    .await;
                assert_eq!(f.server().comm_state(), 0);
                assert!(old.is_finished());
                assert!(timer.lock().await.is_none());
                f.dcc(
                    peer,
                    DISABLE_INITIATION,
                    Some(1),
                    Some(PASSWORD),
                    None,
                    Outcome::Policy,
                )
                .await;
                assert_eq!(f.server().comm_state(), 0);
                assert!(timer.lock().await.is_none());
                f.read_property(peer).await;
            }
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn sc_dcc_mtls_failure_path_joins_fixture() {
    use futures_util::FutureExt;
    use std::panic::AssertUnwindSafe;
    let result = AssertUnwindSafe(run(async |f| {
        let certs = Certificates::new();
        f.hub(&certs).await;
        f.start(&certs, BACnetServer::sc_builder()).await;
        panic!("intentional fixture failure");
    }))
    .catch_unwind()
    .await;
    let panic = result.expect_err("injected failure must propagate after cleanup");
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"intentional fixture failure")
    );
}

#[tokio::test]
async fn sc_dcc_mtls_routed_source_and_existing_duration_semantics() {
    run(async |f| {
        let certs = Certificates::new();
        f.hub(&certs).await;
        let routed = NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[44]),
        };
        let restriction = DccSourceRestriction::new(vec![DccSource::Routed {
            network: 7,
            address: vec![44],
        }])
        .unwrap();
        f.start(
            &certs,
            BACnetServer::sc_builder()
                .dcc_policy(DccPolicy::RequirePassword)
                .dcc_password(PASSWORD)
                .dcc_source_restriction(Some(restriction)),
        )
        .await;
        for source in [
            None,
            Some(NpduAddress {
                network: 8,
                ..routed.clone()
            }),
            Some(NpduAddress {
                mac_address: MacAddr::from_slice(&[45]),
                ..routed.clone()
            }),
        ] {
            f.dcc(
                0,
                DISABLE_INITIATION,
                None,
                Some(PASSWORD),
                source,
                Outcome::Policy,
            )
            .await;
        }
        f.dcc(
            0,
            DISABLE_INITIATION,
            None,
            Some(PASSWORD),
            Some(routed.clone()),
            Outcome::Accepted,
        )
        .await;
        assert_eq!(f.server().comm_state(), 2);
        assert!(f.server().dcc_timer.lock().await.is_none());
        // Matching routed claims through different TLS peers are not principals.
        f.dcc(
            1,
            ENABLE,
            Some(1),
            Some(PASSWORD),
            Some(routed.clone()),
            Outcome::Accepted,
        )
        .await;
        assert_eq!(f.server().comm_state(), 0);
        assert!(f.server().dcc_timer.lock().await.is_some());
        f.dcc(
            0,
            DISABLE_INITIATION,
            Some(0),
            Some(PASSWORD),
            Some(routed),
            Outcome::Accepted,
        )
        .await;
        // Existing zero duration installs a zero-delay timer, rather than making
        // disable indefinite. Join it through the existing private ownership slot.
        let timer = f.server().dcc_timer.clone();
        let mut task = timer.lock().await.take().unwrap();
        let completed = tokio::time::timeout(Duration::from_secs(5), &mut task).await;
        if completed.is_err() {
            task.abort();
            let _ = task.await;
        }
        completed
            .expect("zero-duration timer did not finish")
            .unwrap();
        assert_eq!(f.server().comm_state(), 0);
        f.read_property(0).await;
    })
    .await;
}
