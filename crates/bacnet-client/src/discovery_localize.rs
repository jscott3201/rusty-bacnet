//! The discovery table when the client learns its network's number
//! (#2572): routed rows on that network name local stations, and rows made
//! local under a number that has since changed are no longer known to be.

use super::*;

/// What [`DeviceTable::localize`] did with one routed row on the local
/// network.
#[derive(Debug)]
pub(crate) struct Folded {
    /// The routed row, as it was, taken out.
    pub(crate) routed: DiscoveredDevice,
    /// The local row it became; `None` where the station's local row stays.
    pub(crate) local: Option<DiscoveredDevice>,
    /// The `add_device` placeholder row the local row took the place of.
    pub(crate) placeholder: Option<DiscoveredDevice>,
}

/// Which row wins when the remote network is this link. A real local
/// station wins over its relayed copy. A real relayed row supersedes a
/// local placeholder.
fn use_routed(local: Option<&TableEntry>, routed: &TableEntry) -> bool {
    local.is_none_or(|local| {
        local.segmentation == PeerSegmentation::Placeholder
            && routed.segmentation != PeerSegmentation::Placeholder
    })
}

fn local_device(row: &DiscoveredDevice, mac: &MacAddr) -> DiscoveredDevice {
    DiscoveredDevice {
        mac_address: mac.clone(),
        source_network: None,
        source_address: None,
        ..row.clone()
    }
}

impl DeviceTable {
    /// A read-only view with the same winners and address order as a fold.
    pub(crate) fn localized_rows(&self, own: Option<u16>) -> Vec<DiscoveredDevice> {
        let Some(number @ 1..=0xfffe) = own else {
            return self.all();
        };
        let first = Key::at(number, &[]);
        let mut aliases = self
            .rows
            .range(first..)
            .take_while(|(k, _)| k.network == number)
            .peekable();
        if aliases.peek().is_none() {
            return self.all();
        }
        let mut stations: BTreeMap<_, _> = self
            .rows
            .iter()
            .filter(|(key, _)| key.network != number)
            .map(|(key, row)| (key.clone(), row))
            .collect();
        for (key, row) in aliases {
            let local = Key::at(0, &key.mac);
            if use_routed(self.rows.get(&local), row) {
                stations.insert(local, row);
            }
        }
        stations
            .into_iter()
            .map(|(key, row)| {
                if row.device.source_network == own {
                    local_device(&row.device, &key.mac)
                } else {
                    row.device.clone()
                }
            })
            .collect()
    }

    /// How many rows claim `instance`, counting to two.
    fn claim_count(&self, instance: u32) -> usize {
        self.claims_of(instance).take(2).count()
    }

    /// Take `network` as the local network: each routed row on it is the
    /// local station at its remote MAC. Where that station has a local row
    /// the routed row is taken out, unless that row only holds the
    /// placeholders `add_device` stores and the routed row what a device
    /// announced or a caller supplied: then, as where the station has no
    /// local row, the routed row becomes the local row, keeping its
    /// capabilities, its ages and its segmentation's provenance. Rows made
    /// so are local by the number alone. Network 0 is local already, and
    /// changes nothing. With each fold, the instances whose claimants
    /// changed while two or more claimed them, before or after.
    pub(crate) fn localize(&mut self, network: u16) -> (Vec<Folded>, Vec<u32>) {
        let (mut folded, mut contested) = (Vec::new(), Vec::new());
        if network == 0 {
            return (folded, contested);
        }
        let first = Key {
            network,
            mac: MacAddr::new(),
        };
        let routed: Vec<Key> = self
            .rows
            .range(first..)
            .take_while(|(key, _)| key.network == network)
            .map(|(key, _)| key.clone())
            .collect();
        for key in routed {
            let local = Key::at(0, &key.mac);
            let Some(entry) = self.rows.get(&key) else {
                continue;
            };
            let moving = use_routed(self.rows.get(&local), entry);
            let mut touched = Vec::new();
            touched.push(entry.device.object_identifier.instance_number());
            if moving && let Some(row) = self.rows.get(&local) {
                let other = row.device.object_identifier.instance_number();
                if !touched.contains(&other) {
                    touched.push(other);
                }
            }
            let before: Vec<usize> = touched.iter().map(|i| self.claim_count(*i)).collect();
            let entry = self.rows.remove(&key).expect("looked up above");
            let instance = entry.device.object_identifier.instance_number();
            self.claims.remove(&(instance, key));
            if moving {
                let replaced = self.rows.remove(&local).map(|row| {
                    let old = row.device.object_identifier.instance_number();
                    self.claims.remove(&(old, local.clone()));
                    row.device
                });
                let device = local_device(&entry.device, &local.mac);
                let row = TableEntry {
                    device: device.clone(),
                    segmentation: entry.segmentation,
                    by_number: true,
                };
                self.rows.insert(local.clone(), row);
                self.claims.insert((instance, local));
                folded.push(Folded {
                    routed: entry.device,
                    local: Some(device),
                    placeholder: replaced,
                });
            } else {
                folded.push(Folded {
                    routed: entry.device,
                    local: None,
                    placeholder: None,
                });
            }
            for (instance, before) in touched.into_iter().zip(before) {
                if (before >= 2 || self.claim_count(instance) >= 2)
                    && !contested.contains(&instance)
                {
                    contested.push(instance);
                }
            }
        }
        contested.sort_unstable();
        (folded, contested)
    }

    /// The network's number changed (#2572): drop every row that was local
    /// only because the old number said so, as nothing now says where those
    /// devices are. Each comes back with its next I-Am.
    pub(crate) fn unlocalize(&mut self) -> Purged {
        let stale: Vec<Key> = self
            .rows
            .iter()
            .filter(|(_, entry)| entry.by_number)
            .map(|(key, _)| key.clone())
            .collect();
        let mut purged = Purged::default();
        for key in stale {
            let Some(entry) = self.rows.remove(&key) else {
                continue;
            };
            let instance = entry.device.object_identifier.instance_number();
            self.claims.remove(&(instance, key));
            if self.claims_of(instance).next().is_some() && !purged.contested.contains(&instance) {
                purged.contested.push(instance);
            }
            purged.lost.push(entry.device);
        }
        purged
    }
}
