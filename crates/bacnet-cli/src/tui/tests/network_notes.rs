//! What the shared BBMD survey flags beyond the grid, on the Network screen
//! (#2130): FDT entries near expiry, BDT entries the survey did not follow
//! and why, and BBMDs past the matrix's limit. Frames at 80x24 and 120x40.

use std::net::{Ipv4Addr, SocketAddrV4};

use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::network_frames::{LOCAL, at_ip, bdt_row, fdt_row, registered};
use super::*;
use crate::tui::app::network::View;
use crate::tui::network::{BdtRow, NetworkEvent};
use bacnet_ops::bbmd::{Here, NotFollowed, Probe, Unfollowed};

fn address(ip: [u8; 4], port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(ip), port)
}

/// Three foreign devices read at 15 s: one with 20 s left, ours with 25 s,
/// and one with 32 s, which crosses under 30 s three seconds later.
#[test]
fn an_expiring_fdt_row_is_marked_in_text() {
    let mut app = registered();
    let soon = address([10, 0, 7, 9], 47808);
    let later = address([10, 0, 7, 10], 47808);
    let tables = NetworkEvent::Tables {
        bbmd: at_ip(1),
        local: LOCAL,
        bdt: Ok(vec![bdt_row(1, [255; 4])]),
        fdt: Ok(vec![
            fdt_row(soon, 20),
            fdt_row(LOCAL, 25),
            fdt_row(later, 32),
        ]),
    };
    worker(&mut app, WorkerEvent::Network(tables));
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.network.view, View::Fdt);
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("10.0.7.9:47808         60 s      ! 20 s    expiring"),
        "{text}"
    );
    // Our own entry keeps its name; the `!` still marks it.
    assert!(
        text.contains("10.0.9.5:47808         60 s      ! 25 s    this client"),
        "{text}"
    );
    assert!(
        text.contains("10.0.7.10:47808        60 s      32 s   "),
        "{text}"
    );
    // Three seconds on, the countdown and the marker agree.
    tick(&mut app, at(18));
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("10.0.7.10:47808        60 s      ! 29 s    expiring"),
        "{text}"
    );
    assert_snapshot!("network_fdt_expiring_120x40", text);
    assert_snapshot!("network_fdt_expiring_80x24", screen(&mut app, 80, 24));
}

/// A seed whose BDT lists one peer and an entry for each reason the survey
/// does not follow one, with the reasons the shared check gives.
fn survey_with_skips() -> NetworkEvent {
    let here = Here {
        own: LOCAL,
        broadcast: Ipv4Addr::BROADCAST,
    };
    let unicast = |address| BdtRow {
        address,
        mask: Ipv4Addr::BROADCAST,
    };
    let odd = [
        unicast(address([0, 0, 0, 0], 0)),
        unicast(address([0, 0, 0, 0], 47808)),
        unicast(address([255, 255, 255, 255], 47808)),
        BdtRow {
            address: address([10, 0, 3, 255], 47808),
            mask: Ipv4Addr::new(255, 255, 255, 0),
        },
        unicast(address([224, 0, 0, 1], 47808)),
        unicast(LOCAL),
        unicast(address([127, 0, 0, 1], 47808)),
    ];
    let not_followed: Vec<Unfollowed> = odd
        .iter()
        .map(|row| Unfollowed {
            address: row.address,
            listed_by: at_ip(1),
            why: NotFollowed::check(row, &here).expect("not followed"),
        })
        .collect();
    let mut seed = vec![bdt_row(1, [255; 4]), bdt_row(2, [255; 4])];
    seed.extend(odd);
    NetworkEvent::Survey {
        seed: at_ip(1),
        probes: vec![
            (at_ip(1), Probe::Table(seed)),
            (
                at_ip(2),
                Probe::Table(vec![bdt_row(1, [255; 4]), bdt_row(2, [255; 4])]),
            ),
        ],
        not_followed,
    }
}

#[test]
fn entries_not_followed_are_listed_with_their_reason() {
    let mut app = registered();
    press(&mut app, KeyCode::Char('m'));
    worker(&mut app, WorkerEvent::Network(survey_with_skips()));
    let text = screen(&mut app, 120, 40);
    // They take no grid column: the two BBMDs read agree.
    assert!(
        text.contains("#2 10.0.2.1:47808         uni   uni "),
        "{text}"
    );
    assert!(!text.contains("#3"), "{text}");
    assert!(text.contains("All 2 BDTs read agree"), "{text}");
    assert!(text.contains("Not read on purpose"), "{text}");
    for (entry, reason) in [
        ("0.0.0.0:0", "unresolved"),
        ("0.0.0.0:47808", "unspecified"),
        ("255.255.255.255:47808", "broadcast"),
        ("10.0.3.255:47808", "directed broadcast"),
        ("224.0.0.1:47808", "multicast"),
        ("10.0.9.5:47808", "our own address"),
        ("127.0.0.1:47808", "loopback"),
    ] {
        let line = format!("{entry:<21}  listed by 10.0.1.1:47808         {reason}");
        assert!(text.contains(&line), "{line}: {text}");
    }
    // Not read on purpose is no failure: no `!` and no warning.
    assert!(!text.contains("! "), "{text}");
    assert_snapshot!("network_not_followed_120x40", text);
    // At 80x24 the legend makes room, and a count stands for the rest.
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Not read on purpose"), "{text}");
    assert!(text.contains("+2 more not read on purpose"), "{text}");
    assert!(!text.contains("Row lists column"), "{text}");
    assert_snapshot!("network_not_followed_80x24", text);
    // A new target clears them with the matrix.
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "bbmd 10.0.4.1");
    press(&mut app, KeyCode::Enter);
    assert!(app.network.not_followed.is_empty());
}

/// A survey as the worker reports one: the BBMDs read, then those found
/// past the survey's limit as skipped probes.
fn worker_survey(read: Vec<(SocketAddrV4, Probe)>, not_followed: Vec<Unfollowed>) -> NetworkEvent {
    let mut probes = read;
    let found: Vec<SocketAddrV4> = probes.iter().map(|(bbmd, _)| *bbmd).collect();
    let mut unread = Vec::new();
    for (_, probe) in &probes {
        if let Probe::Table(rows) = probe {
            for row in rows {
                let skipped = not_followed.iter().any(|s| s.address == row.address);
                if !skipped && !found.contains(&row.address) && !unread.contains(&row.address) {
                    unread.push(row.address);
                }
            }
        }
    }
    probes.extend(unread.into_iter().map(|bbmd| (bbmd, Probe::Skipped)));
    NetworkEvent::Survey {
        seed: at_ip(1),
        probes,
        not_followed,
    }
}

/// Sixteen BBMDs read, each listing forty: the other 24 come back skipped,
/// the matrix keeps 32 and omits 8, and the screen counts them with those
/// that don't fit its width.
#[test]
fn bbmds_past_the_grid_limit_are_counted() {
    let mut app = registered();
    press(&mut app, KeyCode::Char('m'));
    let everyone: Vec<BdtRow> = (1..=40).map(|last| bdt_row(last, [255; 4])).collect();
    let read = (1..=16)
        .map(|last| (at_ip(last), Probe::Table(everyone.clone())))
        .collect();
    worker(
        &mut app,
        WorkerEvent::Network(worker_survey(read, Vec::new())),
    );
    let omitted = app.network.matrix.as_ref().map(|(m, _)| m.omitted);
    assert_eq!(omitted, Some(8));
    let text = screen(&mut app, 120, 40);
    // 14 of 32 fit 118 columns: 18 more, and the 8 omitted.
    assert!(text.contains("#14   +26 more"), "{text}");
    assert!(text.contains("Some BDTs were not read."), "{text}");
    assert!(
        text.contains("8 more BBMDs past the grid's limit of 32 are not shown."),
        "{text}"
    );
    assert_snapshot!("network_matrix_omitted_120x40", text);
    let text = screen(&mut app, 80, 24);
    assert!(
        text.contains("8 more BBMDs past the grid's limit of 32 are not shown."),
        "{text}"
    );
    assert_snapshot!("network_matrix_omitted_80x24", text);
}

/// Sixteen BBMDs that agree, and a seed listing seventeen multicast
/// entries besides: the matrix holds 33 and omits one, but that one is an
/// entry not followed, not a BBMD, so nothing reads as unread.
#[test]
fn entries_not_followed_past_the_limit_are_no_omitted_bbmds() {
    let mut app = registered();
    press(&mut app, KeyCode::Char('m'));
    let bbmds: Vec<BdtRow> = (1..=16).map(|last| bdt_row(last, [255; 4])).collect();
    let multicast: Vec<BdtRow> = (1..=17)
        .map(|last| BdtRow {
            address: address([224, 0, 0, last], 47808),
            mask: Ipv4Addr::BROADCAST,
        })
        .collect();
    let not_followed = multicast
        .iter()
        .map(|row| Unfollowed {
            address: row.address,
            listed_by: at_ip(1),
            why: NotFollowed::Multicast,
        })
        .collect();
    let seed = bbmds.iter().chain(&multicast).copied().collect();
    let mut read = vec![(at_ip(1), Probe::Table(seed))];
    read.extend((2..=16).map(|last| (at_ip(last), Probe::Table(bbmds.clone()))));
    worker(
        &mut app,
        WorkerEvent::Network(worker_survey(read, not_followed)),
    );
    let omitted = app.network.matrix.as_ref().map(|(m, _)| m.omitted);
    assert_eq!(omitted, Some(1));
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("All 16 BDTs read agree"), "{text}");
    assert!(text.contains("17 other entries were"), "{text}");
    assert!(!text.contains("Some BDTs were not read"), "{text}");
    assert!(!text.contains("past the grid's limit"), "{text}");
    // 14 of 16 fit 118 columns.
    assert!(text.contains("#14   +2 more"), "{text}");
}

/// At 80x24 with eight BBMDs, seven of them missing their entry back for
/// the seed, and three entries not followed: the first finding shows
/// whole and a count stands for the other six; nothing is cut mid-line.
#[test]
fn findings_come_first_and_count_what_is_cut_80x24() {
    let mut app = registered();
    press(&mut app, KeyCode::Char('m'));
    let odd = [
        address([224, 0, 0, 1], 47808),
        address([255, 255, 255, 255], 47808),
        address([127, 0, 0, 1], 47808),
    ];
    let why = [
        NotFollowed::Multicast,
        NotFollowed::Broadcast,
        NotFollowed::Loopback,
    ];
    let mut seed: Vec<BdtRow> = (1..=8).map(|last| bdt_row(last, [255; 4])).collect();
    seed.extend(odd.iter().map(|&address| BdtRow {
        address,
        mask: Ipv4Addr::BROADCAST,
    }));
    let not_followed = odd
        .iter()
        .zip(why)
        .map(|(&address, why)| Unfollowed {
            address,
            listed_by: at_ip(1),
            why,
        })
        .collect();
    let mut read = vec![(at_ip(1), Probe::Table(seed))];
    read.extend((2..=8).map(|last| (at_ip(last), Probe::Table(vec![bdt_row(last, [255; 4])]))));
    worker(
        &mut app,
        WorkerEvent::Network(worker_survey(read, not_followed)),
    );
    let text = screen(&mut app, 80, 24);
    // Eight BBMDs fit 78 columns with no `+k more` column.
    assert!(
        text.contains("#8 10.0.8.1:47808         MISS  -     -"),
        "{text}"
    );
    assert!(
        text.contains("! 10.0.1.1:47808 lists 10.0.2.1:47808, but 10.0.2.1:47808 does not list"),
        "{text}"
    );
    assert!(
        text.contains("10.0.1.1:47808: broadcasts go one way only"),
        "{text}"
    );
    assert!(text.contains("+6 more findings"), "{text}");
    assert!(!text.contains("10.0.3.1:47808 does not list"), "{text}");
    assert_snapshot!("network_findings_cut_80x24", text);
    // At 120x40 every finding shows, then the entries not followed.
    let text = screen(&mut app, 120, 40);
    assert!(!text.contains("more findings"), "{text}");
    assert!(
        text.contains("10.0.8.1:47808 does not list 10.0.1.1:47808"),
        "{text}"
    );
    assert!(
        text.contains("127.0.0.1:47808        listed by 10.0.1.1:47808         loopback"),
        "{text}"
    );
    assert_snapshot!("network_findings_cut_120x40", text);
}
