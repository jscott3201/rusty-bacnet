//! `set_mode` and the status handle (#938).

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;

use super::*;

fn foreign_to(bbmd_port: u16) -> ForeignDeviceConfig {
    ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port,
        ttl: 60,
        renewal_interval: Some(Duration::from_secs(5)),
    }
}

fn transport() -> BipTransport {
    BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST)
}

/// Each of a BBMD's options reaches the setter it stands for.
#[test]
fn bbmd_mode_maps_every_option_onto_its_setter() {
    let policy = ForeignDevicePolicy {
        max_ttl: 90,
        ..ForeignDevicePolicy::default()
    };
    let fanout = FanoutPolicy {
        max_fanout_per_input: 7,
        queue_capacity: 0,
        ..FanoutPolicy::default()
    };
    let row = BdtEntry {
        ip: [10, 0, 0, 2],
        port: 0xBAC0,
        broadcast_mask: [255; 4],
    };
    let mut transport = transport();
    transport.set_mode(BipMode::Bbmd(BbmdOptions {
        bdt: vec![row.clone()],
        foreign_device_policy: Some(policy.clone()),
        management_acl: vec![[10, 0, 0, 9]],
        bdt_persist_path: Some(PathBuf::from("bdt.bin")),
        fanout_policy: fanout.clone(),
    }));
    let config = transport.bbmd_config.as_ref().unwrap();
    assert_eq!(config.initial_bdt, [row]);
    assert_eq!(config.foreign_device_policy, Some(policy));
    assert_eq!(config.management_acl, [[10, 0, 0, 9]]);
    assert_eq!(
        transport.bdt_persist_path.as_deref(),
        Some(std::path::Path::new("bdt.bin"))
    );
    // Sanitized, as `set_fanout_policy` leaves it.
    assert_eq!(transport.fanout_policy, fanout.sanitized());
    assert!(transport.foreign_device.is_none());
}

/// The foreign mode's configuration, renewal interval included, is the
/// one the transport registers with.
#[test]
fn foreign_mode_keeps_its_renewal_interval() {
    let mut transport = transport();
    transport.set_mode(BipMode::Foreign(foreign_to(47808)));
    let foreign = transport.foreign_device.as_ref().unwrap();
    assert_eq!(foreign.renewal_interval, Some(Duration::from_secs(5)));
    assert_eq!(foreign.ttl, 60);
    assert!(transport.bbmd_config.is_none());
}

/// A mode replaces whatever was set up before it, so the transport is in one
/// mode, and starts in it, whatever the earlier setters did.
#[tokio::test]
async fn a_mode_replaces_the_one_before_it() {
    let mut transport = transport();
    transport.enable_bbmd(Vec::new());
    transport.set_bdt_persist_path(PathBuf::from("bdt.bin"));
    transport.set_fanout_policy(FanoutPolicy {
        max_fanout_per_input: 7,
        ..FanoutPolicy::default()
    });
    transport.register_as_foreign_device(foreign_to(47808));
    transport.set_mode(BipMode::Normal);
    assert!(transport.bbmd_config.is_none());
    assert!(transport.foreign_device.is_none());
    assert!(transport.bdt_persist_path.is_none());
    assert_eq!(transport.fanout_policy, FanoutPolicy::default());
    let _rx = transport.start().await.unwrap();
    assert!(transport.bbmd_state().is_none());
    transport.stop().await.unwrap();

    let mut transport = self::transport();
    transport.set_mode(BipMode::Foreign(foreign_to(47808)));
    transport.set_mode(BipMode::Bbmd(BbmdOptions::default()));
    assert!(transport.foreign_device.is_none());
    let _rx = transport.start().await.unwrap();
    assert!(transport.bbmd_state().is_some());
    transport.stop().await.unwrap();
}

/// The handle reads the registration of a transport another task owns, and
/// sees it cleared when that task stops the transport, even in the moment
/// between the node stopping and the status clearing.
#[tokio::test]
async fn the_status_handle_follows_a_transport_it_does_not_own() {
    let bbmd = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut transport = transport();
    transport.set_mode(BipMode::Foreign(foreign_to(
        bbmd.local_addr().unwrap().port(),
    )));
    let status = transport.bvlc_client_status();
    assert!(!status.snapshot().running);
    let in_the_gap = Arc::new(std::sync::Mutex::new(None));
    let (gap_status, gap) = (status.clone(), Arc::clone(&in_the_gap));
    transport.between_stops = Some(Box::new(move || {
        *gap.lock().unwrap() = Some(gap_status.snapshot());
    }));
    let _rx = transport.start().await.unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let owner = tokio::spawn(async move {
        let _ = stopped.await;
        transport.stop().await.unwrap();
    });
    // Every registration is answered, so one answered late is retried.
    tokio::spawn(async move {
        let mut packet = [0; 64];
        while let Ok((len, from)) = bbmd.recv_from(&mut packet).await {
            if packet[..len] == [0x81, 5, 0, 6, 0, 60] {
                let _ = bbmd.send_to(&[0x81, 0, 0, 6, 0, 0], from).await;
            }
        }
    });

    tokio::time::timeout(Duration::from_secs(20), async {
        while !status.snapshot().foreign_registration.registered {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the handle saw the registration");
    let live = status.snapshot();
    assert!(live.running);
    assert!(live.foreign_registration.lease_remaining.is_some());
    assert!(live.foreign_registration.next_attempt_in.unwrap() <= Duration::from_secs(5));

    stop.send(()).unwrap();
    owner.await.unwrap();
    let gap = in_the_gap.lock().unwrap().take().expect("read in the gap");
    assert!(gap.running);
    assert!(!gap.foreign_registration.registered);
    assert_eq!(gap.foreign_registration.lease_remaining, None);
    let after = status.snapshot();
    assert!(!after.running);
    assert!(!after.foreign_registration.registered);
    assert_eq!(after.foreign_registration.next_attempt_in, None);
    assert!(after.foreign_registration.accepted >= 1);
}
