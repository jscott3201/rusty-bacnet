use super::*;

fn make_bbmd() -> BbmdState {
    let mut state = BbmdState::new([192, 168, 1, 1], 0xBAC0);
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    state
}

#[test]
fn bdt_set_and_get() {
    let mut bbmd = make_bbmd();
    let entries = vec![
        BdtEntry {
            ip: [192, 168, 1, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        },
        BdtEntry {
            ip: [192, 168, 2, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        },
    ];
    bbmd.set_bdt(entries.clone()).unwrap();
    assert_eq!(bbmd.bdt().len(), 2);
    assert_eq!(bbmd.bdt()[0], entries[0]);
}

#[test]
fn bdt_encode_decode_round_trip() {
    let entries = vec![
        BdtEntry {
            ip: [10, 0, 1, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 0],
        },
        BdtEntry {
            ip: [10, 0, 2, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 0],
        },
    ];
    let mut bbmd = make_bbmd();
    bbmd.set_bdt(entries.clone()).unwrap();
    // set_bdt auto-inserts self, so 3 entries total
    assert_eq!(bbmd.bdt().len(), 3);

    let mut buf = BytesMut::new();
    bbmd.encode_bdt(&mut buf);
    assert_eq!(buf.len(), 30); // 3 * 10 bytes

    let decoded = BbmdState::decode_bdt(&buf).unwrap();
    assert_eq!(decoded.len(), 3);
    assert!(decoded.contains(&entries[0]));
    assert!(decoded.contains(&entries[1]));
}

#[test]
fn bdt_decode_invalid_length() {
    assert!(BbmdState::decode_bdt(&[0; 7]).is_err());
}

#[test]
fn set_bdt_rejects_max_entries_when_self_insert_would_exceed_limit() {
    let mut bbmd = make_bbmd();
    let entries = (0..BbmdState::MAX_BDT_ENTRIES)
        .map(|i| BdtEntry {
            ip: [10, 0, (i / 256) as u8, i as u8],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        })
        .collect();

    assert!(bbmd.set_bdt(entries).is_err());
    assert!(bbmd.bdt().is_empty());
}

#[test]
fn register_and_lookup_foreign_device() {
    let mut bbmd = make_bbmd();
    let result = bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);
    assert_eq!(bbmd.fdt().len(), 1);
    assert_eq!(bbmd.fdt()[0].ip, [10, 0, 0, 5]);
    assert_eq!(bbmd.fdt()[0].ttl, 60);
}

#[test]
fn register_foreign_device_zero_ttl_naks() {
    let mut bbmd = make_bbmd();
    let result = bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 0);
    assert_eq!(result, BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);
    assert!(bbmd.fdt().is_empty());
}

#[test]
fn re_register_updates_existing() {
    let mut bbmd = make_bbmd();
    bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
    bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 120);
    assert_eq!(bbmd.fdt().len(), 1);
    assert_eq!(bbmd.fdt()[0].ttl, 120);
}

#[test]
fn delete_foreign_device() {
    let mut bbmd = make_bbmd();
    bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
    let result = bbmd.delete_foreign_device([10, 0, 0, 5], 0xBAC0);
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);
    assert!(bbmd.fdt().is_empty());
}

#[test]
fn delete_nonexistent_foreign_device_naks() {
    let mut bbmd = make_bbmd();
    let result = bbmd.delete_foreign_device([10, 0, 0, 5], 0xBAC0);
    assert_eq!(
        result,
        BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
    );
}

#[test]
fn expired_entries_purged() {
    let mut bbmd = make_bbmd();
    // Insert an entry that's past TTL + grace period (0 + 30 = 30s, elapsed 40s)
    bbmd.fdt.push(FdtEntry {
        ip: [10, 0, 0, 5],
        port: 0xBAC0,
        ttl: 0,
        registered_at: Instant::now() - Duration::from_secs(40),
    });
    assert!(bbmd.fdt().is_empty());
}

#[test]
fn fdt_encode() {
    let mut bbmd = make_bbmd();
    bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
    let mut buf = BytesMut::new();
    bbmd.encode_fdt(&mut buf);
    assert_eq!(buf.len(), FDT_ENTRY_SIZE);
    // IP
    assert_eq!(&buf[0..4], &[10, 0, 0, 5]);
    // Port
    assert_eq!(u16::from_be_bytes([buf[4], buf[5]]), 0xBAC0);
    // TTL
    assert_eq!(u16::from_be_bytes([buf[6], buf[7]]), 60);
}

#[test]
fn fdt_encode_caps_max_ttl_remaining_time() {
    let mut bbmd = make_bbmd();
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
        max_ttl: u16::MAX,
        ..Default::default()
    });
    let result = bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, u16::MAX);
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

    let mut buf = BytesMut::new();
    bbmd.encode_fdt(&mut buf);

    assert_eq!(buf.len(), FDT_ENTRY_SIZE);
    assert_eq!(u16::from_be_bytes([buf[6], buf[7]]), u16::MAX);
    assert_eq!(u16::from_be_bytes([buf[8], buf[9]]), u16::MAX);
}

#[test]
fn forwarding_targets_excludes_source() {
    let mut bbmd = BbmdState::new([192, 168, 1, 1], 0xBAC0);
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());
    bbmd.set_bdt(vec![
        BdtEntry {
            ip: [192, 168, 1, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        },
        BdtEntry {
            ip: [192, 168, 2, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        },
    ])
    .unwrap();
    bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);

    // Source is some device on our subnet (not us and not a BDT peer)
    let targets = bbmd.forwarding_targets([192, 168, 1, 100], 0xBAC0);

    assert_eq!(targets.len(), 2);
    assert!(targets.contains(&([192, 168, 2, 1], 0xBAC0)));
    assert!(targets.contains(&([10, 0, 0, 5], 0xBAC0)));
}

#[test]
fn forwarding_targets_uses_broadcast_mask() {
    let mut bbmd = BbmdState::new([192, 168, 1, 1], 0xBAC0);
    bbmd.set_bdt(vec![
        BdtEntry {
            ip: [192, 168, 1, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 0],
        },
        BdtEntry {
            ip: [192, 168, 2, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 0],
        },
    ])
    .unwrap();

    let targets = bbmd.forwarding_targets([192, 168, 1, 100], 0xBAC0);
    assert_eq!(targets.len(), 1);
    assert!(targets.contains(&([192, 168, 2, 255], 0xBAC0)));
}

#[test]
fn forwarding_targets_unicast_with_full_mask() {
    let mut bbmd = BbmdState::new([192, 168, 1, 1], 0xBAC0);
    bbmd.set_bdt(vec![
        BdtEntry {
            ip: [192, 168, 1, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        },
        BdtEntry {
            ip: [10, 0, 0, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        },
    ])
    .unwrap();

    let targets = bbmd.forwarding_targets([192, 168, 1, 100], 0xBAC0);
    assert_eq!(targets.len(), 1);
    assert!(targets.contains(&([10, 0, 0, 1], 0xBAC0)));
}

#[test]
fn forwarding_targets_excludes_originating_foreign_device_and_expired_entries() {
    let mut bbmd = BbmdState::new([192, 168, 1, 1], 0xBAC0);
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());
    bbmd.set_bdt(vec![BdtEntry {
        ip: [192, 168, 2, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 255],
    }])
    .unwrap();
    bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
    bbmd.fdt.push(FdtEntry {
        ip: [10, 0, 0, 6],
        port: 0xBAC0,
        ttl: 60,
        registered_at: Instant::now() - Duration::from_secs(91),
    });

    let targets = bbmd.forwarding_targets([10, 0, 0, 5], 0xBAC0);

    assert_eq!(targets, vec![([192, 168, 2, 1], 0xBAC0)]);
    assert_eq!(bbmd.fdt().len(), 1);
}

#[test]
fn ttl_accepted_as_is() {
    let mut bbmd = make_bbmd();
    bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 1);
    assert_eq!(bbmd.fdt()[0].ttl, 1);
}

#[test]
fn set_bdt_auto_inserts_self() {
    let mut state = BbmdState::new([192, 168, 1, 1], 47808);
    let entries = vec![BdtEntry {
        ip: [192, 168, 1, 2],
        port: 47808,
        broadcast_mask: [255, 255, 255, 255],
    }];
    state.set_bdt(entries).unwrap();
    assert_eq!(state.bdt().len(), 2);
    assert!(state
        .bdt()
        .iter()
        .any(|e| e.ip == [192, 168, 1, 1] && e.port == 47808));
}

#[test]
fn set_bdt_does_not_duplicate_self() {
    let mut state = BbmdState::new([192, 168, 1, 1], 0xBAC0);
    let entries = vec![BdtEntry {
        ip: [192, 168, 1, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 255],
    }];
    state.set_bdt(entries).unwrap();
    assert_eq!(state.bdt().len(), 1); // self already present, no duplicate
}

#[test]
fn fdt_grace_period() {
    let mut bbmd = make_bbmd();
    // Insert entry that expired based on TTL alone but within grace period
    bbmd.fdt.push(FdtEntry {
        ip: [10, 0, 0, 5],
        port: 0xBAC0,
        ttl: 60,
        registered_at: Instant::now() - Duration::from_secs(70), // 10s past TTL, but within 30s grace
    });
    assert!(
        !bbmd.fdt().is_empty(),
        "should still be alive during grace period"
    );
}

#[test]
fn is_registered_foreign_device_check() {
    let mut bbmd = make_bbmd();
    bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
    assert!(bbmd.is_registered_foreign_device([10, 0, 0, 5], 0xBAC0));
    assert!(!bbmd.is_registered_foreign_device([10, 0, 0, 6], 0xBAC0));
}

#[test]
fn seconds_remaining_includes_grace_period() {
    let mut bbmd = make_bbmd();
    bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
    let remaining = bbmd.fdt()[0].seconds_remaining();
    assert!(
        remaining <= 90, // TTL(60) + grace(30)
        "seconds_remaining ({remaining}) must not exceed TTL+grace (90)"
    );
    assert!(
        remaining > 60,
        "should include grace period (got {remaining})"
    );
}

#[test]
fn management_acl_empty_denies_all() {
    let bbmd = make_bbmd();
    assert!(!bbmd.is_management_allowed(&[10, 0, 0, 1]));
    assert!(!bbmd.is_management_allowed(&[192, 168, 1, 1]));
}

#[test]
fn management_acl_restricts_to_listed_ips() {
    let mut bbmd = make_bbmd();
    bbmd.set_management_acl(vec![[10, 0, 0, 1], [10, 0, 0, 2]]);
    assert!(bbmd.is_management_allowed(&[10, 0, 0, 1]));
    assert!(bbmd.is_management_allowed(&[10, 0, 0, 2]));
    assert!(!bbmd.is_management_allowed(&[10, 0, 0, 3]));
    assert!(!bbmd.is_management_allowed(&[192, 168, 1, 1]));
}

#[test]
fn fdt_decode_round_trip() {
    let mut bbmd = make_bbmd();
    bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
    let mut buf = BytesMut::new();
    bbmd.encode_fdt(&mut buf);

    let entries = decode_fdt(&buf).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].ip, [10, 0, 0, 5]);
    assert_eq!(entries[0].port, 0xBAC0);
    assert_eq!(entries[0].ttl, 60);
    assert!(entries[0].seconds_remaining <= 90);
}

#[test]
fn fdt_decode_invalid_length() {
    assert!(decode_fdt(&[0; 7]).is_err());
}

#[test]
fn encode_bdt_entries_matches_state() {
    let mut entries = vec![
        BdtEntry {
            ip: [10, 0, 1, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 0],
        },
        BdtEntry {
            ip: [10, 0, 2, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 0],
        },
    ];

    let mut buf_state = BytesMut::new();
    let mut bbmd = make_bbmd();
    bbmd.set_bdt(entries.clone()).unwrap();
    bbmd.encode_bdt(&mut buf_state);

    // set_bdt auto-inserts self, so include self in the expected entries
    entries.push(BdtEntry {
        ip: [192, 168, 1, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 255],
    });
    let mut buf_fn = BytesMut::new();
    encode_bdt_entries(&entries, &mut buf_fn);

    assert_eq!(buf_state, buf_fn);
}

#[test]
fn management_acl_clear_denies_all() {
    let mut bbmd = make_bbmd();
    bbmd.set_management_acl(vec![[10, 0, 0, 1]]);
    assert!(!bbmd.is_management_allowed(&[10, 0, 0, 2]));
    bbmd.set_management_acl(Vec::new());
    assert!(!bbmd.is_management_allowed(&[10, 0, 0, 2]));
    assert!(!bbmd.is_management_allowed(&[10, 0, 0, 1]));
}
