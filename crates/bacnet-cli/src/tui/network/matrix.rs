//! The BDT consistency matrix: given the BDT read from each BBMD a survey
//! found, which entries agree and which don't.
//!
//! Row *i*, column *j* says whether BBMD *i*'s BDT lists BBMD *j*. A
//! working set of BBMDs gives a symmetric grid: each BBMD lists itself
//! (Annex J.4.3.2 puts the local BBMD's own entry in its table), each peer
//! that forwards to another is listed back by it, and every table uses one
//! mask for a given BBMD, since a receiving BBMD tells directed broadcasts
//! from unicasts by the mask in its own entry (Annex J.4.5). A broken
//! entry makes broadcasts reach a subnet one way only, which is quiet in
//! the field: nothing errors, devices just go missing.
//!
//! Pure functions only: the worker reads the tables, the view draws the
//! grid.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_transport::bvll::BvlcResultCode;

use super::BdtRow;

/// What reading one BBMD's BDT gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Probe {
    /// Its table.
    Table(Vec<BdtRow>),
    /// It answered with this NAK result.
    Nak(BvlcResultCode),
    /// It did not answer, or the request could not be sent.
    Unreachable(String),
    /// Not read: the survey had reached its limit.
    Skipped,
}

impl Probe {
    fn table(&self) -> Option<&[BdtRow]> {
        match self {
            Self::Table(rows) => Some(rows),
            _ => None,
        }
    }
}

/// One cell of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cell {
    /// The row's BDT lists the column with this mask; `mixed` when other
    /// tables list the column with a different one.
    Listed {
        /// The mask.
        mask: Ipv4Addr,
        /// Not the mask every table uses for this BBMD.
        mixed: bool,
    },
    /// Not listed, and nothing says it should be.
    Absent,
    /// Not listed, but should be: the column lists the row, or this is the
    /// row's own entry.
    Missing,
    /// The row's table could not be read.
    Unknown,
}

/// One problem the matrix shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Finding {
    /// `from` lists `to`, but `to` does not list `from`: broadcasts reach
    /// `to`'s subnet from `from`'s, and not back.
    Asymmetric {
        /// The BBMD whose table has the entry.
        from: SocketAddrV4,
        /// The BBMD whose table lacks the entry back.
        to: SocketAddrV4,
    },
    /// The BBMD's BDT has no entry for itself.
    MissingSelf(SocketAddrV4),
    /// The tables list `peer` with different masks.
    MixedMasks {
        /// The BBMD listed.
        peer: SocketAddrV4,
        /// Every mask used for it, lowest first.
        masks: Vec<Ipv4Addr>,
    },
    /// The BBMD refused the read.
    Nak {
        /// The BBMD.
        bbmd: SocketAddrV4,
        /// The NAK it answered with.
        code: BvlcResultCode,
    },
    /// The BBMD could not be read.
    Unreachable {
        /// The BBMD.
        bbmd: SocketAddrV4,
        /// Why.
        reason: String,
    },
}

impl Finding {
    /// One line for the findings list.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Asymmetric { from, to } => {
                format!(
                    "{from} lists {to}, but {to} does not list {from}: broadcasts go one way only"
                )
            }
            Self::MissingSelf(bbmd) => {
                format!("{bbmd} has no entry for itself (Annex J.4.3.2)")
            }
            Self::MixedMasks { peer, masks } => {
                let masks: Vec<String> = masks.iter().map(Ipv4Addr::to_string).collect();
                format!(
                    "{peer} is listed with different masks: {}",
                    masks.join(", ")
                )
            }
            Self::Nak { bbmd, code } => format!("{bbmd} refused the BDT read ({code})"),
            Self::Unreachable { bbmd, reason } => format!("{bbmd} could not be read ({reason})"),
        }
    }
}

/// The grid and its findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Matrix {
    /// The BBMDs, rows and columns alike, in the order the survey found them.
    pub(crate) bbmds: Vec<SocketAddrV4>,
    /// `cells[row][column]`.
    pub(crate) cells: Vec<Vec<Cell>>,
    /// Every problem found, in the order the view lists them.
    pub(crate) findings: Vec<Finding>,
}

impl Matrix {
    /// Build the grid from what a survey read. A BBMD some table lists but
    /// the survey has no probe for is added as a row that could not be read.
    pub(crate) fn build(probes: &[(SocketAddrV4, Probe)]) -> Self {
        let mut bbmds: Vec<SocketAddrV4> = Vec::new();
        let mut add = |address: SocketAddrV4| {
            if !bbmds.contains(&address) {
                bbmds.push(address);
            }
        };
        for (bbmd, probe) in probes {
            add(*bbmd);
            for row in probe.table().unwrap_or_default() {
                add(row.address);
            }
        }
        let tables: Vec<Option<&[BdtRow]>> = bbmds
            .iter()
            .map(|bbmd| {
                probes
                    .iter()
                    .find(|(address, _)| address == bbmd)
                    .and_then(|(_, probe)| probe.table())
            })
            .collect();
        // The mask each table gives a BBMD: the first entry for it.
        let listed = |row: usize, column: usize| -> Option<Ipv4Addr> {
            tables[row]?
                .iter()
                .find(|entry| entry.address == bbmds[column])
                .map(|entry| entry.mask)
        };
        let n = bbmds.len();
        let mut findings = Vec::new();
        for (bbmd, probe) in probes {
            match probe {
                Probe::Nak(code) => findings.push(Finding::Nak {
                    bbmd: *bbmd,
                    code: *code,
                }),
                Probe::Unreachable(reason) => findings.push(Finding::Unreachable {
                    bbmd: *bbmd,
                    reason: reason.clone(),
                }),
                Probe::Table(_) | Probe::Skipped => {}
            }
        }
        let mut mixed = vec![false; n];
        for (column, flag) in mixed.iter_mut().enumerate() {
            let masks: BTreeSet<Ipv4Addr> = (0..n).filter_map(|row| listed(row, column)).collect();
            if masks.len() > 1 {
                *flag = true;
                findings.push(Finding::MixedMasks {
                    peer: bbmds[column],
                    masks: masks.into_iter().collect(),
                });
            }
        }
        let mut cells = vec![vec![Cell::Unknown; n]; n];
        for row in 0..n {
            if tables[row].is_none() {
                continue;
            }
            for column in 0..n {
                cells[row][column] = match listed(row, column) {
                    Some(mask) => Cell::Listed {
                        mask,
                        mixed: mixed[column],
                    },
                    None if row == column => {
                        findings.push(Finding::MissingSelf(bbmds[row]));
                        Cell::Missing
                    }
                    None if listed(column, row).is_some() => {
                        findings.push(Finding::Asymmetric {
                            from: bbmds[column],
                            to: bbmds[row],
                        });
                        Cell::Missing
                    }
                    None => Cell::Absent,
                };
            }
        }
        Self {
            bbmds,
            cells,
            findings,
        }
    }

    /// True when every table was read and nothing disagrees.
    pub(crate) fn consistent(&self) -> bool {
        self.findings.is_empty()
            && self
                .cells
                .iter()
                .flatten()
                .all(|cell| *cell != Cell::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNICAST: Ipv4Addr = Ipv4Addr::BROADCAST;
    const SUBNET: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);

    fn at(last: u8) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(10, 0, last, 1), 47808)
    }

    fn table(rows: &[(u8, Ipv4Addr)]) -> Probe {
        Probe::Table(
            rows.iter()
                .map(|(last, mask)| BdtRow {
                    address: at(*last),
                    mask: *mask,
                })
                .collect(),
        )
    }

    fn listed(mask: Ipv4Addr) -> Cell {
        Cell::Listed { mask, mixed: false }
    }

    #[test]
    fn symmetric_tables_are_consistent() {
        let both = table(&[(1, UNICAST), (2, UNICAST)]);
        let matrix = Matrix::build(&[(at(1), both.clone()), (at(2), both)]);
        assert_eq!(matrix.bbmds, [at(1), at(2)]);
        assert!(matrix.findings.is_empty(), "{:?}", matrix.findings);
        assert!(matrix.consistent());
        assert!(matrix.cells.iter().flatten().all(|c| *c == listed(UNICAST)));
    }

    #[test]
    fn a_one_way_entry_is_flagged_on_the_side_that_lacks_it() {
        let matrix = Matrix::build(&[
            (at(1), table(&[(1, UNICAST), (2, UNICAST)])),
            (at(2), table(&[(2, UNICAST)])),
        ]);
        // A lists B; B lacks A.
        assert_eq!(matrix.cells[0][1], listed(UNICAST));
        assert_eq!(matrix.cells[1][0], Cell::Missing);
        assert_eq!(
            matrix.findings,
            [Finding::Asymmetric {
                from: at(1),
                to: at(2)
            }]
        );
        assert!(!matrix.consistent());
        assert!(matrix.findings[0].describe().contains("one way"));
    }

    #[test]
    fn a_table_without_its_own_entry_is_flagged() {
        let matrix = Matrix::build(&[
            (at(1), table(&[(2, UNICAST)])),
            (at(2), table(&[(1, UNICAST), (2, UNICAST)])),
        ]);
        assert_eq!(matrix.cells[0][0], Cell::Missing);
        assert_eq!(matrix.findings, [Finding::MissingSelf(at(1))]);
    }

    #[test]
    fn two_masks_for_one_bbmd_are_flagged_in_its_column() {
        let matrix = Matrix::build(&[
            (at(1), table(&[(1, UNICAST), (2, SUBNET)])),
            (at(2), table(&[(1, UNICAST), (2, UNICAST)])),
        ]);
        let mixed = |mask| Cell::Listed { mask, mixed: true };
        assert_eq!(matrix.cells[0][1], mixed(SUBNET));
        assert_eq!(matrix.cells[1][1], mixed(UNICAST));
        assert_eq!(matrix.cells[0][0], listed(UNICAST));
        assert_eq!(
            matrix.findings,
            [Finding::MixedMasks {
                peer: at(2),
                masks: vec![SUBNET, UNICAST]
            }]
        );
    }

    #[test]
    fn unread_peers_are_unknown_and_named() {
        let matrix = Matrix::build(&[
            (at(1), table(&[(1, UNICAST), (2, UNICAST), (3, UNICAST)])),
            (
                at(2),
                Probe::Nak(BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK),
            ),
            (at(3), Probe::Unreachable("no answer".into())),
        ]);
        assert_eq!(matrix.cells[1], [Cell::Unknown; 3]);
        assert_eq!(matrix.cells[2], [Cell::Unknown; 3]);
        // Nothing is known of their tables, so no asymmetry is claimed.
        assert_eq!(matrix.cells[0], [listed(UNICAST); 3]);
        assert_eq!(matrix.findings.len(), 2);
        assert!(matches!(matrix.findings[0], Finding::Nak { bbmd, .. } if bbmd == at(2)));
        assert!(matches!(
            matrix.findings[1],
            Finding::Unreachable { bbmd, .. } if bbmd == at(3)
        ));
        assert!(!matrix.consistent());
    }

    #[test]
    fn a_listed_bbmd_without_a_probe_is_a_row_that_was_not_read() {
        let matrix = Matrix::build(&[(at(1), table(&[(1, UNICAST), (4, UNICAST)]))]);
        assert_eq!(matrix.bbmds, [at(1), at(4)]);
        assert_eq!(matrix.cells[1], [Cell::Unknown; 2]);
        assert!(matrix.findings.is_empty());
        assert!(!matrix.consistent(), "an unread table is not consistent");
    }

    #[test]
    fn peers_that_list_neither_way_are_absent_not_missing() {
        let matrix = Matrix::build(&[
            (at(1), table(&[(1, UNICAST), (2, UNICAST), (3, UNICAST)])),
            (at(2), table(&[(1, UNICAST), (2, UNICAST)])),
            (at(3), table(&[(1, UNICAST), (3, UNICAST)])),
        ]);
        assert_eq!(matrix.cells[1][2], Cell::Absent);
        assert_eq!(matrix.cells[2][1], Cell::Absent);
        assert!(matrix.consistent());
    }
}
