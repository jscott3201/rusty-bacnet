//! The verdict over tables already read: findings name the addresses, and
//! every quantity is decoded (addresses as ip:port, masks dotted, times in
//! seconds).

use std::net::Ipv4Addr;

use bacnet_ops::bbmd::{BdtRow, NotFollowed, Surveyed, Unfollowed};

use super::*;

const ONES: Ipv4Addr = Ipv4Addr::BROADCAST;
const SUBNET: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);

fn at(last: u8) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(10, 0, last, 1), 47808)
}

fn bdt(rows: &[(u8, Ipv4Addr)]) -> Probe {
    Outcome::Table(
        rows.iter()
            .map(|&(last, mask)| BdtRow {
                address: at(last),
                mask,
            })
            .collect(),
    )
}

fn read(bbmd: SocketAddrV4, probe: Probe, fdt: Vec<FdtRow>) -> Surveyed<Fdt> {
    Surveyed {
        bbmd,
        probe,
        extra: Some(Outcome::Table(fdt)),
    }
}

/// The report of a survey that read `surveyed` and nothing more.
fn report(seeds: &[SocketAddrV4], surveyed: &[Surveyed<Fdt>]) -> Value {
    super::report(
        seeds,
        &Survey {
            read: surveyed.to_vec(),
            unread: Vec::new(),
            not_followed: Vec::new(),
        },
    )
}

fn problems(report: &Value) -> Vec<&str> {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["problem"].as_str().unwrap())
        .collect()
}

#[test]
fn agreeing_tables_are_consistent() {
    let both = bdt(&[(1, ONES), (2, ONES)]);
    let report = report(
        &[at(1)],
        &[read(at(1), both.clone(), vec![]), read(at(2), both, vec![])],
    );
    assert_eq!(report["verdict"], json!("consistent"));
    assert_eq!(report["findings"], json!([]));
    assert_eq!(report["seeds"], json!(["10.0.1.1:47808"]));
    assert_eq!(
        report["bbmds"][0]["bdt"]["entries"][1],
        json!({ "address": "10.0.2.1:47808", "mask": "255.255.255.255",
                "forward_to": "10.0.2.1" })
    );
}

#[test]
fn mismatched_masks_name_the_bbmd_and_both_masks() {
    let report = report(
        &[at(1)],
        &[
            read(at(1), bdt(&[(1, ONES), (2, SUBNET)]), vec![]),
            read(at(2), bdt(&[(1, ONES), (2, ONES)]), vec![]),
        ],
    );
    assert_eq!(report["verdict"], json!("inconsistent"));
    assert_eq!(problems(&report), ["mismatched-masks"]);
    let finding = &report["findings"][0];
    assert_eq!(finding["bbmd"], json!("10.0.2.1:47808"));
    assert_eq!(
        finding["masks"],
        json!(["255.255.255.0", "255.255.255.255"])
    );
    assert_eq!(
        report["bbmds"][0]["bdt"]["entries"][1]["forward_to"],
        json!("10.0.2.255")
    );
}

#[test]
fn a_one_way_entry_names_both_ends() {
    let report = report(
        &[at(1)],
        &[
            read(at(1), bdt(&[(1, ONES), (2, ONES)]), vec![]),
            read(at(2), bdt(&[(2, ONES)]), vec![]),
        ],
    );
    assert_eq!(report["verdict"], json!("inconsistent"));
    assert_eq!(problems(&report), ["one-way-entry"]);
    assert_eq!(report["findings"][0]["from"], json!("10.0.1.1:47808"));
    assert_eq!(report["findings"][0]["to"], json!("10.0.2.1:47808"));
}

/// An entry under 30 s left is past its time-to-live; one at 30 s isn't
/// flagged yet.
#[test]
fn a_foreign_device_near_expiry_is_flagged_in_seconds() {
    let fdt = vec![
        FdtRow {
            address: at(7),
            ttl: 60,
            remaining: 12,
        },
        FdtRow {
            address: at(8),
            ttl: 60,
            remaining: 30,
        },
    ];
    let report = report(&[at(1)], &[read(at(1), bdt(&[(1, ONES)]), fdt)]);
    assert_eq!(problems(&report), ["foreign-device-expiring"]);
    let finding = &report["findings"][0];
    assert_eq!(finding["bbmd"], json!("10.0.1.1:47808"));
    assert_eq!(finding["device"], json!("10.0.7.1:47808"));
    assert_eq!(
        (finding["ttl_s"].clone(), finding["remaining_s"].clone()),
        (json!(60), json!(12))
    );
    // The tables agree: the verdict is about broadcast distribution.
    assert_eq!(report["verdict"], json!("consistent"));
    assert_eq!(report["bbmds"][0]["fdt"]["count"], json!(2));
}

/// A NAK is "doesn't support reading", a silence "no answer": neither is an
/// inconsistency, and the verdict says the survey is incomplete.
#[test]
fn a_nak_and_a_silence_leave_the_survey_incomplete() {
    let code = BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK;
    let report = report(
        &[at(1)],
        &[
            read(at(1), bdt(&[(1, ONES), (2, ONES), (3, ONES)]), vec![]),
            Surveyed {
                bbmd: at(2),
                probe: Outcome::Nak(code),
                extra: None,
            },
            Surveyed {
                bbmd: at(3),
                probe: Outcome::Unreachable(NO_ANSWER.into()),
                extra: None,
            },
        ],
    );
    assert_eq!(report["verdict"], json!("incomplete"));
    assert_eq!(problems(&report), ["read-not-supported", "no-answer"]);
    assert_eq!(
        report["findings"][0]["result_code"],
        json!({ "value": 0x20, "name": "read-broadcast-distribution-table-nak" })
    );
    assert_eq!(report["bbmds"][1]["bdt"]["status"], json!("not-supported"));
    assert_eq!(report["bbmds"][1]["fdt"]["status"], json!("not-read"));
    assert_eq!(report["bbmds"][2]["bdt"]["status"], json!("no-answer"));
}

/// A BBMD whose FDT read is refused is reported for that table alone.
#[test]
fn an_fdt_nak_is_reported_for_the_fdt() {
    let code = BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK;
    let report = report(
        &[at(1)],
        &[Surveyed {
            bbmd: at(1),
            probe: bdt(&[(1, ONES)]),
            extra: Some(Outcome::Nak(code)),
        }],
    );
    assert_eq!(report["verdict"], json!("consistent"));
    assert_eq!(problems(&report), ["read-not-supported"]);
    assert_eq!(report["findings"][0]["table"], json!("fdt"));
}

/// Long tables list their first entries and say they were cut; the check
/// still sees them all.
#[test]
fn long_tables_are_cut_and_counted() {
    let fdt = (0..40)
        .map(|n| FdtRow {
            address: SocketAddrV4::new(Ipv4Addr::new(10, 9, 0, n), 47808),
            ttl: 60,
            remaining: 5,
        })
        .collect();
    let report = report(&[at(1)], &[read(at(1), bdt(&[(1, ONES)]), fdt)]);
    assert_eq!(report["truncated"], json!(true));
    assert_eq!(report["bbmds"][0]["fdt"]["count"], json!(40));
    assert_eq!(
        report["bbmds"][0]["fdt"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        LISTED
    );
    assert_eq!(report["finding_count"], json!(40));
}

/// An entry not followed is reported with its reason, and the BBMDs past
/// the survey's limit as a count with a few examples; both leave the
/// survey incomplete and the result marked cut.
#[test]
fn unfollowed_and_unread_bbmds_are_reported_bounded() {
    let unread: Vec<SocketAddrV4> = (0..2_000u32)
        .map(|n| SocketAddrV4::new(Ipv4Addr::from(0x0B00_0000 + n), 47808))
        .collect();
    let found = Survey {
        read: vec![read(at(1), bdt(&[(1, ONES)]), vec![])],
        unread,
        not_followed: vec![Unfollowed {
            address: "0.0.0.0:0".parse().unwrap(),
            listed_by: at(1),
            why: NotFollowed::Unresolved,
        }],
    };
    let report = super::report(&[at(1)], &found);
    assert_eq!(report["verdict"], json!("incomplete"));
    assert_eq!(problems(&report), ["not-followed"]);
    assert_eq!(report["findings"][0]["reason"], json!("unresolved"));
    assert_eq!(report["findings"][0]["address"], json!("0.0.0.0:0"));
    assert_eq!(report["unread"]["count"], json!(2_000));
    assert_eq!(
        report["unread"]["examples"].as_array().unwrap().len(),
        UNREAD_EXAMPLES
    );
    assert_eq!(report["truncated"], json!(true));
    assert!(report.to_string().len() < 10_000);
}

/// An empty BDT (a BBMD for foreign devices only) is a note; the verdict
/// stays consistent.
#[test]
fn an_empty_table_is_a_note() {
    let report = report(&[at(1)], &[read(at(1), bdt(&[]), vec![])]);
    assert_eq!(report["verdict"], json!("consistent"));
    assert_eq!(problems(&report), ["no-peers"]);
}
