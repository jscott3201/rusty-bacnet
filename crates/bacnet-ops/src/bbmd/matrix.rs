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
//! Pure functions only: a front end reads the tables (the TUI's network
//! worker, the MCP server's network map) and shows the grid or findings.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_transport::bvll::BvlcResultCode;

use super::{BdtRow, Outcome};

/// What reading one BBMD's BDT gave.
pub type Probe = Outcome<Vec<BdtRow>>;

/// One cell of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
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
pub enum Finding {
    /// `from` lists `to`, but `to` does not list `from`: broadcasts reach
    /// `to`'s subnet from `from`'s, and not back.
    Asymmetric {
        /// The BBMD whose table has the entry.
        from: SocketAddrV4,
        /// The BBMD whose table lacks the entry back.
        to: SocketAddrV4,
    },
    /// The BBMD's BDT has entries, but none for itself.
    MissingSelf(SocketAddrV4),
    /// The BBMD's BDT is empty: it forwards broadcasts to no peer, as a
    /// BBMD that serves only foreign devices may. A note, not a problem.
    NoPeers(SocketAddrV4),
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
    pub fn describe(&self) -> String {
        match self {
            Self::Asymmetric { from, to } => {
                format!(
                    "{from} lists {to}, but {to} does not list {from}: broadcasts go one way only"
                )
            }
            Self::MissingSelf(bbmd) => {
                format!("{bbmd} has no entry for itself (Annex J.4.3.2)")
            }
            Self::NoPeers(bbmd) => {
                format!("{bbmd} has an empty BDT: it forwards to no peer BBMD")
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

/// The most BBMDs the grid shows, rows and columns alike; the rest are
/// counted in [`Matrix::omitted`]. The findings cover them all.
pub const GRID_CAP: usize = 32;

/// The grid and its findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matrix {
    /// The BBMDs, rows and columns alike: those probed in the order the
    /// survey found them, then those only listed, at most [`GRID_CAP`].
    pub bbmds: Vec<SocketAddrV4>,
    /// `cells[row][column]`.
    pub cells: Vec<Vec<Cell>>,
    /// BBMDs left out of the grid past [`GRID_CAP`].
    pub omitted: usize,
    /// Every problem found, in the order the view lists them.
    pub findings: Vec<Finding>,
}

impl Matrix {
    /// Build the grid from what a survey read. A BBMD some table lists but
    /// the survey has no probe for is added as a row that could not be read.
    ///
    /// The work is bounded by what was read, not by what the tables list:
    /// the findings compare the tables read with one another, and the grid
    /// stops at [`GRID_CAP`] BBMDs.
    pub fn build(probes: &[(SocketAddrV4, Probe)]) -> Self {
        let mut seen = HashSet::new();
        let mut bbmds: Vec<SocketAddrV4> = Vec::new();
        for (bbmd, _) in probes {
            if seen.insert(*bbmd) {
                bbmds.push(*bbmd);
            }
        }
        for (_, probe) in probes {
            for row in probe.table().map_or(&[][..], Vec::as_slice) {
                if seen.insert(row.address) {
                    bbmds.push(row.address);
                }
            }
        }
        // Each BBMD's table, from its first probe.
        let mut tables: HashMap<SocketAddrV4, &[BdtRow]> = HashMap::new();
        for (bbmd, probe) in probes {
            if let Some(rows) = probe.table() {
                tables.entry(*bbmd).or_insert(rows.as_slice());
            }
        }
        // The mask a table gives a BBMD: its first entry for it.
        let listed = |row: SocketAddrV4, column: SocketAddrV4| -> Option<Ipv4Addr> {
            tables
                .get(&row)?
                .iter()
                .find(|entry| entry.address == column)
                .map(|entry| entry.mask)
        };
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
        // Every mask the tables give each BBMD.
        let mut masks: HashMap<SocketAddrV4, BTreeSet<Ipv4Addr>> = HashMap::new();
        for rows in tables.values() {
            let mut first = HashSet::new();
            for entry in rows.iter().filter(|entry| first.insert(entry.address)) {
                masks.entry(entry.address).or_default().insert(entry.mask);
            }
        }
        let mixed: HashSet<SocketAddrV4> = bbmds
            .iter()
            .copied()
            .filter(|bbmd| masks.get(bbmd).is_some_and(|set| set.len() > 1))
            .collect();
        for bbmd in bbmds.iter().filter(|bbmd| mixed.contains(bbmd)) {
            findings.push(Finding::MixedMasks {
                peer: *bbmd,
                masks: masks[bbmd].iter().copied().collect(),
            });
        }
        // The tables read, compared with one another.
        let read: Vec<SocketAddrV4> = bbmds
            .iter()
            .copied()
            .filter(|bbmd| tables.contains_key(bbmd))
            .collect();
        for &row in &read {
            for &column in &read {
                if listed(row, column).is_some() {
                    continue;
                }
                if row == column {
                    findings.push(match tables[&row].is_empty() {
                        true => Finding::NoPeers(row),
                        false => Finding::MissingSelf(row),
                    });
                } else if listed(column, row).is_some() {
                    findings.push(Finding::Asymmetric {
                        from: column,
                        to: row,
                    });
                }
            }
        }
        let omitted = bbmds.len().saturating_sub(GRID_CAP);
        bbmds.truncate(GRID_CAP);
        let n = bbmds.len();
        let mut cells = vec![vec![Cell::Unknown; n]; n];
        for (r, &row) in bbmds.iter().enumerate() {
            let Some(table) = tables.get(&row) else {
                continue;
            };
            for (c, &column) in bbmds.iter().enumerate() {
                cells[r][c] = match listed(row, column) {
                    Some(mask) => Cell::Listed {
                        mask,
                        mixed: mixed.contains(&column),
                    },
                    None if row == column && !table.is_empty() => Cell::Missing,
                    None if row != column && listed(column, row).is_some() => Cell::Missing,
                    None => Cell::Absent,
                };
            }
        }
        Self {
            bbmds,
            cells,
            omitted,
            findings,
        }
    }

    /// True when every table was read, the grid shows every BBMD and
    /// nothing disagrees (an empty table is a note, not a disagreement).
    pub fn consistent(&self) -> bool {
        self.omitted == 0
            && self
                .findings
                .iter()
                .all(|f| matches!(f, Finding::NoPeers(_)))
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

    /// An empty table (a BBMD for foreign devices only) is a note, and the
    /// grid stays consistent; a peer listing it still makes a one-way entry.
    #[test]
    fn an_empty_table_is_a_note_not_a_problem() {
        let matrix = Matrix::build(&[(at(1), table(&[]))]);
        assert_eq!(matrix.findings, [Finding::NoPeers(at(1))]);
        assert_eq!(matrix.cells, [[Cell::Absent]]);
        assert!(matrix.consistent());
        let matrix = Matrix::build(&[
            (at(1), table(&[(1, UNICAST), (2, UNICAST)])),
            (at(2), table(&[])),
        ]);
        assert_eq!(
            matrix.findings,
            [
                Finding::Asymmetric {
                    from: at(1),
                    to: at(2)
                },
                Finding::NoPeers(at(2)),
            ]
        );
        assert!(!matrix.consistent());
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
