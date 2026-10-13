use super::*;

#[tokio::test]
async fn registration_token_follows_last_socket_worker_after_abort() {
    let token = Arc::new(());
    let weak = Arc::downgrade(&token);
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.retain_network_port_lease_internal(token).unwrap();
    assert!(transport
        .retain_network_port_lease_internal(Arc::new(()))
        .is_err());
    let _received = transport.start().await.unwrap();
    let address = transport.bip_port().unwrap().endpoint;
    assert_ne!(address.port(), 0);
    let socket = transport.socket.as_ref().unwrap().clone();
    // Transport, receive, fanout and this held worker all share the same owner.
    assert!(Arc::strong_count(&socket) >= 4);
    let (release, held) = oneshot::channel::<()>();
    let worker = tokio::spawn(async move {
        let _ = held.await;
        drop(socket);
    });
    let aborted = transport.abort_background_tasks();
    drop(transport);
    for task in aborted {
        let _ = task.await;
    }
    assert!(weak.upgrade().is_some());
    assert!(std::net::UdpSocket::bind(address).is_err());
    release.send(()).unwrap();
    worker.await.unwrap();
    assert!(weak.upgrade().is_none());
    let _reused = std::net::UdpSocket::bind(address).unwrap();
}

#[tokio::test]
async fn normal_capability_and_joined_stop_release_are_explicit() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    assert!(transport.supports_local_nonrouter_number_controls());
    assert!(matches!(
        transport.bip_port().unwrap().mode,
        BipPortMode::Normal
    ));
    let token = Arc::new(());
    let weak = Arc::downgrade(&token);
    transport.retain_network_port_lease_internal(token).unwrap();
    let _received = transport.start().await.unwrap();
    assert!(transport.supports_local_nonrouter_number_controls());
    assert!(transport
        .retain_network_port_lease_internal(Arc::new(()))
        .is_err());
    transport.stop().await.unwrap();
    assert!(weak.upgrade().is_none());
    assert!(transport
        .retain_network_port_lease_internal(Arc::new(()))
        .is_err());
}

/// A BBMD reports its mode before start, takes the lease, and lends its own
/// live tables once started: a later table change is what the view reads.
#[tokio::test]
async fn bbmd_reports_its_mode_and_lends_its_live_tables_once_started() {
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd.enable_bbmd(vec![]);
    assert!(bbmd.supports_local_nonrouter_number_controls());
    let staged = bbmd.bip_port().unwrap();
    assert_eq!(staged.endpoint, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
    assert!(matches!(staged.mode, BipPortMode::Bbmd { tables: None }));
    let token = Arc::new(());
    let weak = Arc::downgrade(&token);
    bbmd.retain_network_port_lease_internal(token).unwrap();
    let _received = bbmd.start().await.unwrap();
    let started = bbmd.bip_port().unwrap();
    assert_eq!(
        started.endpoint,
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, bbmd.port)
    );
    let BipPortMode::Bbmd {
        tables: Some(tables),
    } = started.mode
    else {
        panic!("a started BBMD lends its tables: {:?}", started.mode);
    };
    let own = BACnetHostNPort::from_socket_addr(started.endpoint.into());
    let rows = tables.broadcast_distribution_table();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].bbmd_address, own);
    assert!(!tables.accepts_foreign_device_registrations());
    assert!(tables.foreign_device_table().is_empty());

    let peer = BdtEntry {
        ip: [10, 0, 0, 2],
        port: 0xBAC0,
        broadcast_mask: [0xFF; 4],
    };
    {
        let mut state = bbmd.bbmd_state().unwrap().lock().unwrap();
        state.set_bdt(vec![peer]).unwrap();
        state.set_foreign_device_policy(Some(ForeignDevicePolicy::default()));
        assert_eq!(
            state.register_foreign_device([10, 0, 0, 9], 0xBAC0, 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }
    let rows = tables.broadcast_distribution_table();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].bbmd_address,
        BACnetHostNPort::from_socket_addr(SocketAddrV4::new([10, 0, 0, 2].into(), 0xBAC0).into())
    );
    assert_eq!(rows[0].broadcast_mask, Some([0xFF; 4]));
    assert_eq!(rows[1].bbmd_address, own);
    assert!(tables.accepts_foreign_device_registrations());
    let fdt = tables.foreign_device_table();
    assert_eq!(fdt.len(), 1);
    assert_eq!(fdt[0].time_to_live, 60);

    bbmd.stop().await.unwrap();
    assert!(weak.upgrade().is_none());
    assert!(bbmd
        .retain_network_port_lease_internal(Arc::new(()))
        .is_err());
}

#[tokio::test]
async fn foreign_device_reports_its_bbmd_and_lifetime() {
    let mut foreign = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    foreign.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::new(10, 0, 0, 1),
        bbmd_port: 47809,
        renewal_interval: None,
        ttl: 60,
    });
    assert!(foreign.supports_local_nonrouter_number_controls());
    let BipPortMode::Foreign {
        bbmd,
        subscription_lifetime,
    } = foreign.bip_port().unwrap().mode
    else {
        panic!("foreign mode");
    };
    assert_eq!(
        bbmd,
        BACnetHostNPort::from_socket_addr(SocketAddrV4::new([10, 0, 0, 1].into(), 47809).into())
    );
    assert_eq!(subscription_lifetime, 60);
    foreign
        .retain_network_port_lease_internal(Arc::new(()))
        .unwrap();
}

#[test]
fn a_bbmd_that_also_registers_as_a_foreign_device_has_no_single_mode() {
    let mut both = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    both.enable_bbmd(vec![]);
    both.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::new(10, 0, 0, 1),
        bbmd_port: 47808,
        renewal_interval: None,
        ttl: 60,
    });
    assert!(both.bip_port().is_none());
    assert!(both
        .retain_network_port_lease_internal(Arc::new(()))
        .is_err());
}

fn foreign_config() -> ForeignDeviceConfig {
    ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::new(10, 0, 0, 1),
        bbmd_port: 47808,
        renewal_interval: None,
        ttl: 60,
    }
}

fn assert_refused_as_both(error: Error) {
    assert!(matches!(error, Error::Encoding(_)), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("a BBMD that also registers as a foreign device"),
        "{error}"
    );
}

/// A transport is in one B/IP mode (#938): one set up both as a BBMD and as
/// a foreign device fails to start, before it binds or sends anything, in
/// either order of setup.
#[tokio::test]
async fn a_bbmd_that_also_registers_as_a_foreign_device_fails_to_start() {
    for bbmd_first in [true, false] {
        // A nonlocal interface would fail the start later, at the bind.
        let mut both = BipTransport::new(Ipv4Addr::new(192, 0, 2, 1), 0, Ipv4Addr::BROADCAST);
        if bbmd_first {
            both.enable_bbmd(vec![]);
            both.register_as_foreign_device(foreign_config());
        } else {
            both.register_as_foreign_device(foreign_config());
            both.enable_bbmd(vec![]);
        }
        assert_refused_as_both(both.start().await.unwrap_err());
        assert!(both.socket.is_none());
        assert!(!both.bvlc_client_snapshot().running);
    }
}

/// The BBMD a transport started as stays across a stop, so registering it as
/// a foreign device afterwards fails the next start too.
#[tokio::test]
async fn a_started_bbmd_cant_restart_as_a_foreign_device() {
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd.enable_bbmd(vec![]);
    let _rx = bbmd.start().await.unwrap();
    bbmd.stop().await.unwrap();
    bbmd.register_as_foreign_device(foreign_config());
    assert_refused_as_both(bbmd.start().await.unwrap_err());
    assert!(bbmd.socket.is_none());
}
