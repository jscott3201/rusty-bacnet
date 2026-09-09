use super::*;

#[tokio::test]
async fn dcc_disable_rate_validation_before_start() {
    assert!(ServerConfig::default().dcc_disable_rate_limit.is_none());
    for limit in [
        DccDisableRateLimit {
            capacity: 0,
            ..Default::default()
        },
        DccDisableRateLimit {
            capacity: 65536,
            ..Default::default()
        },
        DccDisableRateLimit {
            capacity: u32::MAX,
            ..Default::default()
        },
        DccDisableRateLimit {
            refill_interval_ms: 0,
            ..Default::default()
        },
        DccDisableRateLimit {
            refill_interval_ms: 86_400_001,
            ..Default::default()
        },
        DccDisableRateLimit {
            refill_interval_ms: u64::MAX,
            ..Default::default()
        },
    ] {
        assert!(limit.validate().is_err());
        let started = Arc::new(AtomicBool::new(false));
        let config = ServerConfig {
            dcc_disable_rate_limit: Some(limit),
            ..Default::default()
        };
        let error = BACnetServer::start(config, ObjectDatabase::new(), NeverStart(started.clone()))
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("DCC disable rate")));
        assert!(BACnetServer::generic_builder()
            .transport(NeverStart(started.clone()))
            .dcc_disable_rate_limit(Some(limit))
            .build()
            .await
            .is_err());
        assert!(!started.load(Ordering::Acquire));
        let error = BACnetServer::bip_builder()
            .dcc_disable_rate_limit(Some(limit))
            .build()
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("DCC disable rate")));
    }
    for limit in [
        DccDisableRateLimit {
            capacity: 1,
            refill_interval_ms: 1,
        },
        DccDisableRateLimit {
            capacity: 65535,
            refill_interval_ms: 86_400_000,
        },
    ] {
        assert!(limit.validate().is_ok());
    }
}

#[cfg(feature = "sc-tls")]
#[tokio::test]
async fn dcc_disable_rate_validation_before_sc_dial() {
    for limit in [
        DccDisableRateLimit {
            capacity: 0,
            ..Default::default()
        },
        DccDisableRateLimit {
            refill_interval_ms: u64::MAX,
            ..Default::default()
        },
    ] {
        let tls = tokio_rustls::rustls::ClientConfig::builder()
            .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
            .with_no_client_auth();
        let error = BACnetServer::sc_builder()
            .hub_url("not-a-websocket-url")
            .tls_config(Arc::new(tls))
            .dcc_disable_rate_limit(Some(limit))
            .build()
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("DCC disable rate")));
    }
}

#[test]
fn dcc_source_restriction_configuration_bounds() {
    for length in [0, 256, 65536] {
        assert!(DccSourceRestriction::new(vec![DccSource::Direct(vec![1; length])]).is_err());
    }
    for network in [0, 65535] {
        assert!(DccSourceRestriction::new(vec![DccSource::Routed {
            network,
            address: vec![1]
        }])
        .is_err());
    }
    assert!(DccSourceRestriction::new(vec![DccSource::Direct(vec![1]); 257]).is_err());
    assert!(DccSourceRestriction::new(vec![DccSource::Direct(vec![1; 255]); 256]).is_ok());
    for entries in [vec![], vec![DccSource::Direct(vec![1])]] {
        let restriction = DccSourceRestriction::new(entries).unwrap();
        assert!(restriction
            .validate_policy(DccPolicy::RequirePassword)
            .is_ok());
        for policy in [DccPolicy::DenyAll, DccPolicy::LegacyPermissive] {
            assert!(restriction.validate_policy(policy).is_err());
        }
    }
}

#[tokio::test]
async fn dcc_source_restriction_rejected_before_start() {
    for policy in [DccPolicy::DenyAll, DccPolicy::LegacyPermissive] {
        let restriction = Some(DccSourceRestriction::new(vec![]).unwrap());
        let started = Arc::new(AtomicBool::new(false));
        let config = ServerConfig {
            dcc_policy: policy,
            dcc_source_restriction: restriction.clone(),
            ..Default::default()
        };
        let error = BACnetServer::start(config, ObjectDatabase::new(), NeverStart(started.clone()))
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("source restriction")));
        assert!(BACnetServer::generic_builder()
            .transport(NeverStart(started.clone()))
            .dcc_policy(policy)
            .dcc_source_restriction(restriction.clone())
            .build()
            .await
            .is_err());
        assert!(!started.load(Ordering::Acquire));
        let error = BACnetServer::bip_builder()
            .dcc_policy(policy)
            .dcc_source_restriction(restriction)
            .build()
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("source restriction")));
    }
}

#[cfg(feature = "sc-tls")]
#[tokio::test]
async fn dcc_source_restriction_rejected_before_sc_dial() {
    for policy in [DccPolicy::DenyAll, DccPolicy::LegacyPermissive] {
        let tls = tokio_rustls::rustls::ClientConfig::builder()
            .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
            .with_no_client_auth();
        let error = BACnetServer::sc_builder()
            .hub_url("not-a-websocket-url")
            .tls_config(Arc::new(tls))
            .dcc_policy(policy)
            .dcc_source_restriction(Some(DccSourceRestriction::new(vec![]).unwrap()))
            .build()
            .await
            .err()
            .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("source restriction")));
    }
}

#[tokio::test]
async fn dcc_source_denied_enable_still_occupies_recovery() {
    let (mut server, _, mut started) = fixture_with_config(
        "source denial",
        ServerConfig {
            dcc_policy: DccPolicy::RequirePassword,
            dcc_password: Some("required".into()),
            dcc_source_restriction: Some(DccSourceRestriction::new(vec![]).unwrap()),
            ..Default::default()
        },
    )
    .await;
    server.comm_state.store(2, Ordering::Release);
    for id in 1..=2 {
        dispatch(&server, enable(id, Some("required")), source(id), None).await;
        observed(&mut started).await;
        assert_eq!(server.comm_state(), 2);
        assert!(server.dcc_timer.lock().await.is_none());
        assert!(
            matches!(server.network.transport().frames.lock().unwrap().last(), Some(Apdu::Error(e))
            if e.error_class == ErrorClass::SERVICES && e.error_code == ErrorCode::SERVICE_REQUEST_DENIED)
        );
    }
    assert_eq!(server.request_admission_counters().recovery_active, 2);
    assert_eq!(
        server.request_admission_counters().recovery_admitted_total,
        2
    );
    assert_eq!(server.dcc_outcome_counters().policy_denied_total, 2);
    server.stop().await.unwrap();
}

#[test]
fn dcc_configured_validation_retains_decode_password_unknown_mode_precedence() {
    for policy in [
        DccPolicy::DenyAll,
        DccPolicy::RequirePassword,
        DccPolicy::LegacyPermissive,
    ] {
        let state = AtomicU8::new(2);
        let password = Some("required".to_owned());
        let invalid = handlers::handle_device_communication_control_with_policy(
            &[0x19],
            &state,
            &password,
            policy,
        );
        assert!(matches!(invalid, Err(Error::Decoding { .. })));
        let unknown = &[0x19, 3];
        let error = handlers::handle_device_communication_control_with_policy(
            unknown, &state, &password, policy,
        );
        assert!(matches!(error, Err(Error::Protocol { class, code })
            if class == ErrorClass::SECURITY.to_raw() as u32 && code == ErrorCode::PASSWORD_FAILURE.to_raw() as u32));
        let mut data = BytesMut::new();
        DeviceCommunicationControlRequest {
            time_duration: None,
            enable_disable: EnableDisable::from_raw(3),
            password: password.clone(),
        }
        .encode(&mut data)
        .unwrap();
        let error = handlers::handle_device_communication_control_with_policy(
            &data, &state, &password, policy,
        );
        assert!(matches!(error, Err(Error::Encoding(m)) if m == "unknown EnableDisable value"));
        assert_eq!(state.load(Ordering::Acquire), 2);
    }
}

pub(super) async fn legacy_fixture() -> (
    BACnetServer<HeldTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::UnboundedReceiver<oneshot::Receiver<()>>,
) {
    fixture_with_config(
        "recovery",
        ServerConfig {
            segmentation_supported: Segmentation::BOTH,
            dcc_policy: DccPolicy::LegacyPermissive,
            ..Default::default()
        },
    )
    .await
}

#[tokio::test]
async fn dcc_require_password_rejected_before_start() {
    for password in [None, Some(String::new())] {
        let started = Arc::new(AtomicBool::new(false));
        let config = ServerConfig {
            dcc_policy: DccPolicy::RequirePassword,
            dcc_password: password.clone(),
            ..Default::default()
        };
        let error = BACnetServer::start(
            config,
            ObjectDatabase::new(),
            NeverStart(Arc::clone(&started)),
        )
        .await
        .err()
        .unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("nonempty dcc_password")));
        let mut builder = BACnetServer::generic_builder()
            .transport(NeverStart(Arc::clone(&started)))
            .dcc_policy(DccPolicy::RequirePassword);
        if let Some(password) = &password {
            builder = builder.dcc_password(password);
        }
        assert!(builder.build().await.is_err());
        assert!(!started.load(Ordering::Acquire));
        let mut builder = BACnetServer::bip_builder().dcc_policy(DccPolicy::RequirePassword);
        if let Some(password) = &password {
            builder = builder.dcc_password(password);
        }
        assert!(builder.build().await.is_err());
    }
    for policy in [DccPolicy::DenyAll, DccPolicy::LegacyPermissive] {
        for password in [None, Some(String::new()), Some("x".repeat(100))] {
            assert!(policy.validate(&password).is_ok());
        }
    }
    let config = ServerConfig {
        dcc_password: Some("secret-sentinel".into()),
        ..Default::default()
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains("secret-sentinel"));
    assert!(debug.contains("DenyAll"));
}

#[cfg(feature = "sc-tls")]
#[tokio::test]
async fn dcc_require_password_rejected_before_sc_dial() {
    for password in [None, Some("")] {
        let tls = tokio_rustls::rustls::ClientConfig::builder()
            .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
            .with_no_client_auth();
        let mut builder = BACnetServer::sc_builder()
            .hub_url("not-a-websocket-url")
            .tls_config(Arc::new(tls))
            .dcc_policy(DccPolicy::RequirePassword);
        if let Some(password) = password {
            builder = builder.dcc_password(password);
        }
        let error = builder.build().await.err().unwrap();
        assert!(matches!(error, Error::Encoding(m) if m.contains("nonempty dcc_password")));
    }
}

#[tokio::test]
async fn dcc_default_recovery_admission_does_not_authorize_enable() {
    for password in [None, Some("required")] {
        let (mut server, _tx, mut started) = fixture_with_config(
            "default deny",
            ServerConfig {
                dcc_password: password.map(str::to_owned),
                ..Default::default()
            },
        )
        .await;
        server.comm_state.store(2, Ordering::Release);
        for id in 1..=2 {
            dispatch(&server, enable(id, password), source(id), None).await;
            observed(&mut started).await;
            assert_eq!(server.comm_state(), 2);
            assert!(server.dcc_timer.lock().await.is_none());
            assert!(
                matches!(server.network.transport().frames.lock().unwrap().last(), Some(Apdu::Error(e))
                if e.error_class == ErrorClass::SERVICES && e.error_code == ErrorCode::SERVICE_REQUEST_DENIED)
            );
        }
        let counters = server.request_admission_counters();
        assert_eq!(counters.recovery_admitted_total, 2);
        assert_eq!(counters.recovery_active, 2);
        server.stop().await.unwrap();
    }
}
