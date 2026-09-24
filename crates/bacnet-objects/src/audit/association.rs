//! Target-only association authority; selector membership is independent of filters.
use super::*;
use std::{collections::HashSet, sync::Mutex};

#[doc(hidden)]
#[derive(Clone)]
pub struct SelectedAuditReporter {
    pub identifier: ObjectIdentifier,
    pub status: Arc<AuditReporterStatus>,
    pub configuration: AuditReporterConfiguration,
}

/// Canonical configured target set and a database-owned subject membership index.
/// Configuration remains in each object's shared status authority, never mirrored.
#[doc(hidden)]
pub struct TargetAuditAssociation {
    reporters: Vec<(ObjectIdentifier, Arc<AuditReporterStatus>)>,
    subjects: Mutex<HashSet<ObjectIdentifier>>,
}
impl TargetAuditAssociation {
    pub fn new(reporters: Vec<(ObjectIdentifier, Arc<AuditReporterStatus>)>) -> Arc<Self> {
        Arc::new(Self {
            reporters,
            subjects: Mutex::new(HashSet::new()),
        })
    }
    pub fn reporters(&self) -> &[(ObjectIdentifier, Arc<AuditReporterStatus>)] {
        &self.reporters
    }
    pub fn snapshots(
        &self,
        replacement: Option<(ObjectIdentifier, &AuditReporterConfiguration)>,
    ) -> Vec<SelectedAuditReporter> {
        self.reporters
            .iter()
            .map(|(identifier, status)| SelectedAuditReporter {
                identifier: *identifier,
                status: Arc::clone(status),
                configuration: replacement
                    .filter(|(oid, _)| oid == identifier)
                    .map_or_else(|| status.configuration(), |(_, value)| value.clone()),
            })
            .collect()
    }
    pub fn select(
        &self,
        target: Option<ObjectIdentifier>,
        kind: ObjectType,
        replacement: Option<(ObjectIdentifier, &AuditReporterConfiguration)>,
    ) -> Option<SelectedAuditReporter> {
        self.snapshots(replacement).into_iter().find(|r| {
            r.configuration.enabled()
                && target.map_or_else(
                    || r.configuration.monitors_unassigned(kind),
                    |target| r.configuration.monitors(target),
                )
        })
    }
    /// Mandatory self fallback is not nominal selector membership or overlap.
    pub fn select_change(
        &self,
        target: ObjectIdentifier,
        replacement: Option<(ObjectIdentifier, &AuditReporterConfiguration)>,
    ) -> Option<SelectedAuditReporter> {
        self.select(Some(target), target.object_type(), replacement)
            .or_else(|| {
                self.snapshots(replacement)
                    .into_iter()
                    .find(|r| r.identifier == target && r.configuration.enabled())
            })
    }
    /// Local pair owner: Device match, then enabled configured, then lowest configured.
    pub fn select_recipient_change(&self, device: ObjectIdentifier) -> SelectedAuditReporter {
        self.select(Some(device), ObjectType::DEVICE, None)
            .or_else(|| {
                self.snapshots(None)
                    .into_iter()
                    .find(|r| r.configuration.enabled())
            })
            .unwrap_or_else(|| self.snapshots(None).remove(0))
    }
    pub(crate) fn set_subjects(&self, subjects: impl IntoIterator<Item = ObjectIdentifier>) {
        *self.subjects.lock().unwrap() = subjects.into_iter().collect();
        self.refresh_overlap();
    }
    pub(crate) fn membership_changed(&self, oid: ObjectIdentifier, present: bool) {
        let mut subjects = self.subjects.lock().unwrap();
        if present {
            subjects.insert(oid);
        } else {
            subjects.remove(&oid);
        }
        drop(subjects);
        self.refresh_overlap();
    }
    pub fn refresh_overlap(&self) {
        let snapshots = self.snapshots(None);
        let subjects = self.subjects.lock().unwrap();
        let mut overlaps = vec![false; snapshots.len()];
        for subject in subjects.iter() {
            let matching: Vec<_> = snapshots
                .iter()
                .enumerate()
                .filter(|(_, r)| r.configuration.enabled() && r.configuration.monitors(*subject))
                .map(|(i, _)| i)
                .collect();
            if matching.len() > 1 {
                for index in matching {
                    overlaps[index] = true;
                }
            }
        }
        for (reporter, overlap) in snapshots.iter().zip(overlaps) {
            reporter.status.set_overlap(overlap);
        }
    }
}
