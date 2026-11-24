//! Rebuild a validated endpoint registration from its saved parameters.

use super::*;
use bacnet_objects::audit::AuditReporterObject;

impl PendingObject {
    pub(super) fn identity(&self) -> (ObjectType, u32, &str) {
        match self {
            Self::Point(point) => point.identity(),
            Self::Group { instance, name, .. } => (ObjectType::GROUP, *instance, name),
            Self::AuditReporter { instance, name, .. } => {
                (ObjectType::AUDIT_REPORTER, *instance, name)
            }
        }
    }

    /// The checks a registration passes before it is built: a point's
    /// (#2711); a Group and an Audit Reporter check as they build.
    pub(super) fn check(&self) -> PyResult<()> {
        match self {
            Self::Point(point) => point.check(),
            Self::Group { .. } | Self::AuditReporter { .. } => Ok(()),
        }
    }

    /// Builds the object, the same way at registration and at each start.
    /// A point builds as `BACnetServer` builds it, without storage (#2711).
    pub(crate) fn build(&self) -> PyResult<Box<dyn BACnetObject>> {
        match self {
            Self::Point(point) => Ok(point.build(Storage::NONE)?),
            Self::Group {
                instance,
                name,
                members,
            } => Ok(Box::new(group_members::group(*instance, name, members)?)),
            Self::AuditReporter {
                instance,
                name,
                level,
                operations,
                confirmed,
            } => {
                let mut object = AuditReporterObject::new(*instance, name).map_err(to_py_err)?;
                object
                    .audit_reporter_mut()
                    .ok_or_else(|| PyValueError::new_err("Audit Reporter lacks its capability"))?
                    .configure(
                        *level,
                        *operations,
                        *confirmed,
                        None,
                        BACnetPriorityFilter::all(),
                        None,
                    )
                    .map_err(to_py_err)?;
                Ok(Box::new(object))
            }
        }
    }
}
