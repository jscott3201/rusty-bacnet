//! `Tables::serve`: the answer to each management request a BBMD serves
//! from its tables, with its result code (Annex J.2.1, J.4.4.1, J.5.2.1.1,
//! J.5.2.1.2, J.5.2.2, J.5.2.3).

mod support;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntry, BvlcResultCode, Bvll, FdtEntry};
use bacnet_bip_core::Instant;
use bacnet_bip_core::bbmd::{
    Answer, DeleteRefusal, Registered, RegistrationLimits, RegistrationRefusal, Request,
    Reservation, Rules, Served, StorageFull, TWO_HOP_MASK, Tables,
};
use bacnet_bip_core::node::Permissive;
use support::*;

/// A foreign device.
const DEVICE: SocketAddrV4 = addr(203, 0, 113, 7, PORT);
/// Another one.
const OTHER: SocketAddrV4 = addr(203, 0, 113, 8, PORT);
/// A management station.
const ADMIN: SocketAddrV4 = addr(192, 168, 1, 50, PORT);
/// A peer BBMD.
const PEER: SocketAddrV4 = addr(10, 0, 2, 1, PORT);

/// An unused row.
const NO_ROW: FdtEntry = FdtEntry::new(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), 0, 0);

const SUCCESS: BvlcResultCode = BvlcResultCode::SUCCESSFUL_COMPLETION;

/// Rows for a Read-Foreign-Device-Table-Ack of [`Small`].
fn rows() -> [FdtEntry; 8] {
    [NO_ROW; 8]
}

/// The BVLC-Result code of `answer`, which must be one.
fn code(answer: &Answer<'_>) -> BvlcResultCode {
    match answer.message {
        Bvll::BvlcResult(code) => code,
        other => panic!("not a result: {other:?}"),
    }
}

/// Serve `request` at `now` with every default rule.
fn serve(tables: &mut Small, request: Request, now: Instant) -> (BvlcResultCode, Served) {
    let mut rows = rows();
    let answer = tables.serve(request, now, &mut Permissive, &mut rows);
    assert_eq!(answer.to, request.sender(), "the requester gets the answer");
    (code(&answer), answer.served)
}

/// A Register-Foreign-Device the table accepts draws Successful completion,
/// a renewal too; any refusal draws the Register-Foreign-Device NAK, 0x0030,
/// and stores nothing (J.5.2.2).
#[test]
fn registrations_draw_success_or_the_register_nak() {
    let mut tables = tables(&[]);
    let now = Instant::from_secs(10);
    let register = |ttl| Request::Register {
        sender: DEVICE,
        ttl,
    };
    let (answer, served) = serve(&mut tables, register(60), now);
    assert_eq!(answer, SUCCESS);
    let Served::Register(outcome) = served else {
        panic!("{served:?}");
    };
    assert_eq!(outcome.result, Ok(Registered::New));
    assert!(tables.fdt.is_registered_at(DEVICE, now));

    let (answer, served) = serve(&mut tables, register(120), now);
    assert_eq!(answer, SUCCESS);
    assert!(matches!(
        served,
        Served::Register(outcome) if outcome.result == Ok(Registered::Renewed)
    ));
    assert_eq!(tables.fdt.get(DEVICE).map(|entry| entry.ttl), Some(120));

    let nak = BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
    let zero = Request::Register {
        sender: OTHER,
        ttl: 0,
    };
    let (answer, served) = serve(&mut tables, zero, now);
    assert_eq!(answer, nak);
    assert!(matches!(
        served,
        Served::Register(outcome) if outcome.result == Err(RegistrationRefusal::TimeToLive)
    ));
    assert_eq!(tables.fdt.get(OTHER), None);

    // A full table refuses a newcomer with the same NAK.
    let mut full: Tables<2, 1> = Tables::new(OWN);
    let mut rows = [NO_ROW; 1];
    let accepted = full.serve(register(60), now, &mut Permissive, &mut rows);
    assert_eq!(code(&accepted), SUCCESS);
    let other = Request::Register {
        sender: OTHER,
        ttl: 60,
    };
    let refused = full.serve(other, now, &mut Permissive, &mut rows);
    assert_eq!(code(&refused), nak);
    assert!(matches!(
        refused.served,
        Served::Register(outcome) if outcome.result == Err(RegistrationRefusal::TableFull)
    ));
}

/// The host's rules for registration: its limits, which sources the
/// reserved slots are for, and an admission check asked only once the
/// time-to-live has passed. Each is counted, so a test sees when it ran.
#[derive(Debug, Default)]
struct Strict {
    admitted: Vec<SocketAddrV4>,
    deletes: Vec<(SocketAddrV4, SocketAddrV4)>,
}

impl Rules for Strict {
    fn registration_limits(&mut self) -> RegistrationLimits {
        limits(30, 600, 1, 1)
    }

    fn reservation(&mut self, source: SocketAddrV4) -> Reservation {
        if source == ADMIN {
            Reservation::Reserved
        } else {
            Reservation::Ordinary
        }
    }

    fn admit_registration(&mut self, source: SocketAddrV4, _now: Instant) -> bool {
        self.admitted.push(source);
        source != OTHER
    }

    fn admit_delete(&mut self, sender: SocketAddrV4, entry: SocketAddrV4) -> bool {
        self.deletes.push((sender, entry));
        sender == ADMIN
    }
}

#[test]
fn the_hosts_rules_hold_registrations_and_deletions() {
    let now = Instant::from_secs(10);
    let mut tables: Tables<2, 2> = Tables::new(OWN);
    let mut rows = [NO_ROW; 2];
    let mut rules = Strict::default();
    let mut serve = |tables: &mut Tables<2, 2>, request, rules: &mut Strict| {
        let answer = tables.serve(request, now, rules, &mut rows);
        (code(&answer), answer.served)
    };
    let register = |sender, ttl| Request::Register { sender, ttl };
    let refusal = |served: Served| match served {
        Served::Register(outcome) => outcome.result.err(),
        other => panic!("{other:?}"),
    };

    // Under the host's minimum: refused before the admission check.
    let (answer, served) = serve(&mut tables, register(DEVICE, 20), &mut rules);
    assert_eq!(answer, BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);
    assert_eq!(refusal(served), Some(RegistrationRefusal::TimeToLive));
    assert!(rules.admitted.is_empty(), "never asked");
    // The admission check says no.
    let (_, served) = serve(&mut tables, register(OTHER, 60), &mut rules);
    assert_eq!(refusal(served), Some(RegistrationRefusal::NotAdmitted));
    // One slot of two is reserved: an ordinary source takes the other, and
    // the next ordinary source finds none.
    let (answer, _) = serve(&mut tables, register(DEVICE, 60), &mut rules);
    assert_eq!(answer, SUCCESS);
    let second = addr(198, 51, 100, 9, PORT);
    let (_, served) = serve(&mut tables, register(second, 60), &mut rules);
    assert_eq!(refusal(served), Some(RegistrationRefusal::TableFull));
    // The reserved source takes the reserved slot.
    let (answer, _) = serve(&mut tables, register(ADMIN, 60), &mut rules);
    assert_eq!(answer, SUCCESS);
    assert_eq!(rules.admitted, [OTHER, DEVICE, second, ADMIN]);

    // Only the host's management station may delete.
    let delete = |sender| Request::DeleteFdtEntry {
        sender,
        entry: DEVICE,
    };
    let (answer, served) = serve(&mut tables, delete(DEVICE), &mut rules);
    assert_eq!(
        answer,
        BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
    );
    assert_eq!(
        served,
        Served::DeleteFdtEntry(Err(DeleteRefusal::NotAdmitted))
    );
    assert!(tables.fdt.is_registered_at(DEVICE, now), "unchanged");
    let (answer, served) = serve(&mut tables, delete(ADMIN), &mut rules);
    assert_eq!(answer, SUCCESS);
    assert_eq!(served, Served::DeleteFdtEntry(Ok(())));
    assert_eq!(rules.deletes, [(DEVICE, DEVICE), (ADMIN, DEVICE)]);
}

/// A Read-Broadcast-Distribution-Table draws the ACK with every row, this
/// BBMD's own included (J.4.4.1); a table never set, an empty list (J.2.4).
#[test]
fn read_bdt_answers_with_every_row() {
    let mut empty = Small::new(OWN);
    let mut rows = rows();
    let answer = empty.serve(
        Request::ReadBdt { sender: ADMIN },
        Instant::ZERO,
        &mut Permissive,
        &mut rows,
    );
    let Bvll::ReadBroadcastDistributionTableAck(entries) = answer.message else {
        panic!("{answer:?}");
    };
    assert_eq!((entries.len(), answer.served), (0, Served::ReadBdt));

    let peer = row(PEER, MASK_24);
    let mut tables = tables(&[peer]);
    let answer = tables.serve(
        Request::ReadBdt { sender: ADMIN },
        Instant::ZERO,
        &mut Permissive,
        &mut rows,
    );
    assert_eq!(answer.to, ADMIN);
    let Bvll::ReadBroadcastDistributionTableAck(entries) = answer.message else {
        panic!("{answer:?}");
    };
    let listed: Vec<BdtEntry> = entries.iter().collect();
    assert_eq!(listed, [peer, row(OWN, TWO_HOP_MASK)]);
}

/// A Read-Foreign-Device-Table draws the ACK with each live registration,
/// its time-to-live and the seconds it has left, the 30 s grace included
/// (J.2.8.1, J.5.2.1.1). A lapsed one is left out. When the rows can't
/// hold them, the read fails with the Read-Foreign-Device-Table NAK, 0x0040.
#[test]
fn read_fdt_answers_with_each_live_registration() {
    let mut tables = tables(&[]);
    register_all(&mut tables, &[DEVICE], 60, Instant::from_secs(10));
    register_all(&mut tables, &[OTHER], 5, Instant::from_secs(10));
    // At 50 s: DEVICE has 60 + 30 - 40 = 50 s left; OTHER lapsed at 45 s.
    let now = Instant::from_secs(50);
    let mut rows = rows();
    let answer = tables.serve(
        Request::ReadFdt { sender: ADMIN },
        now,
        &mut Permissive,
        &mut rows,
    );
    let Bvll::ReadForeignDeviceTableAck(entries) = answer.message else {
        panic!("{answer:?}");
    };
    let listed: Vec<FdtEntry> = entries.iter().collect();
    assert_eq!(listed, [FdtEntry::new(DEVICE, 60, 50)]);
    assert_eq!(answer.served, Served::ReadFdt(Ok(1)));

    // Two live registrations and one row: the read fails.
    register_all(&mut tables, &[OTHER], 60, now);
    let mut one = [NO_ROW; 1];
    let answer = tables.serve(
        Request::ReadFdt { sender: ADMIN },
        now,
        &mut Permissive,
        &mut one,
    );
    assert_eq!(code(&answer), BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK);
    assert_eq!(
        answer.served,
        Served::ReadFdt(Err(StorageFull {
            needed: 2,
            available: 1
        }))
    );
}

/// Delete-Foreign-Device-Table-Entry removes a live registration, with
/// Successful completion; with none, it draws the Delete NAK, 0x0050
/// (J.5.2.1.2). A lapsed registration counts as none, and goes too.
#[test]
fn delete_fdt_entry_removes_a_live_registration_or_naks() {
    let mut tables = tables(&[]);
    register_all(&mut tables, &[DEVICE], 60, Instant::from_secs(10));
    register_all(&mut tables, &[OTHER], 5, Instant::from_secs(10));
    let now = Instant::from_secs(50);
    let delete = |entry| Request::DeleteFdtEntry {
        sender: ADMIN,
        entry,
    };
    let nak = BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK;

    assert_eq!(
        serve(&mut tables, delete(DEVICE), now),
        (SUCCESS, Served::DeleteFdtEntry(Ok(())))
    );
    assert_eq!(tables.fdt.get(DEVICE), None);
    let refused = (
        nak,
        Served::DeleteFdtEntry(Err(DeleteRefusal::NotRegistered)),
    );
    assert_eq!(serve(&mut tables, delete(DEVICE), now), refused, "gone");
    assert!(tables.fdt.get(OTHER).is_some(), "lapsed, not yet purged");
    assert_eq!(serve(&mut tables, delete(OTHER), now), refused, "lapsed");
    assert!(tables.fdt.is_empty(), "the lapsed one went too");
    assert_eq!(DeleteRefusal::NotAdmitted.result_code(), nak);
}

#[test]
fn each_request_names_its_sender() {
    for request in [
        Request::Register {
            sender: DEVICE,
            ttl: 1,
        },
        Request::ReadBdt { sender: DEVICE },
        Request::ReadFdt { sender: DEVICE },
        Request::DeleteFdtEntry {
            sender: DEVICE,
            entry: OTHER,
        },
    ] {
        assert_eq!(request.sender(), DEVICE);
    }
}
