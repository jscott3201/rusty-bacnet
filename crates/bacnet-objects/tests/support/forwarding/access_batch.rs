//! A distinct batch answer for the shared wrapper forwarding probe.

use super::{AccessEventReport, CommittedAccessEvent, Error, Probe};
use bacnet_objects::access_control::AccessPointObject;

pub(super) fn record(
    probe: &Probe,
    reports: &[AccessEventReport],
) -> Result<Vec<CommittedAccessEvent>, Error> {
    probe.called("record_access_event_batch_internal", (reports,));
    reports
        .iter()
        .cloned()
        .map(|report| {
            let mut point = AccessPointObject::new(9, "batch probe")?;
            point.set_access_event(report.clone())?;
            Ok(CommittedAccessEvent {
                report,
                snapshot: Box::new(point),
                clock_frame: super::super::fixtures::clock().read_clock(),
            })
        })
        .collect()
}
