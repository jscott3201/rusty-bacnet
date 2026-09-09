use super::*;

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
