//! Paired optional delay capability and object-owned flush command state.
use super::*;
use crate::device::AuditWriteSource;

/// Audit Reporter Maximum_Send_Delay, in whole seconds (0 through 3600).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditSendDelay(u16);
impl AuditSendDelay {
    /// Validate the Standard's inclusive range; zero keeps the pair present.
    pub fn new(seconds: u32) -> Result<Self, Error> {
        if seconds > 3600 {
            return Err(crate::common::value_out_of_range_error());
        }
        Ok(Self(seconds as u16))
    }
    /// Configured whole seconds, with zero meaning no additional delay.
    pub fn seconds(self) -> u16 {
        self.0
    }
}

impl AuditReporterObject {
    /// Author the optional delay/Send_Now pair, or change a live delay value.
    /// Presence is fixed throughout an active target ownership lifetime.
    pub fn set_maximum_send_delay(&mut self, delay: Option<AuditSendDelay>) -> Result<(), Error> {
        let mut next = self.configuration_internal();
        next.maximum_send_delay = delay;
        self.change_configuration(next, None)
    }

    /// Flush the currently delayed prefix. FALSE clears readback and continues
    /// already-owned work; a new TRUE command establishes a new completion fence.
    pub fn set_send_now(&mut self, value: bool) -> Result<(), Error> {
        self.command_send_now(value, None)
    }

    fn command_send_now(
        &mut self,
        value: bool,
        source: Option<&AuditWriteSource>,
    ) -> Result<(), Error> {
        if self.configuration_internal().maximum_send_delay.is_none() {
            return Err(crate::common::unknown_property_error());
        }
        if let Some(sink) = self.change_sink.as_ref().and_then(std::sync::Weak::upgrade) {
            if !sink.is_active() {
                return Err(crate::common::write_access_denied_error());
            }
            return sink.send_now(
                self.oid,
                &self.status,
                value,
                source,
                self.clock.as_ref().and_then(|clock| clock.read_clock()),
            );
        }
        if self.change_owner.upgrade().is_some() {
            return Err(crate::common::write_access_denied_error());
        }
        // An unowned object has no delayed notifications.
        self.status.commit_send_now(value, 0, false)?;
        Ok(())
    }
}

impl AuditReporterAuthority<'_> {
    /// Write one of the concrete writable Reporter fields with original provenance.
    pub fn write_property(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
        index: Option<u32>,
        source: Option<&AuditWriteSource>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::DESCRIPTION {
            return self.write_description(value, index, source);
        }
        if !matches!(
            property,
            PropertyIdentifier::MAXIMUM_SEND_DELAY | PropertyIdentifier::SEND_NOW
        ) {
            return Err(crate::common::write_access_denied_error());
        }
        let mut next = self.0.configuration_internal();
        if next.maximum_send_delay.is_none() {
            return Err(crate::common::unknown_property_error());
        }
        if index.is_some() {
            return Err(crate::common::property_is_not_an_array_error());
        }
        if matches!(value, PropertyValue::Null) {
            // Equal configuration still checks sealed ownership before no-op success.
            return self.0.change_configuration(next, source);
        }
        match (property, value) {
            (PropertyIdentifier::MAXIMUM_SEND_DELAY, PropertyValue::Unsigned(seconds)) => {
                let seconds = u32::try_from(seconds)
                    .map_err(|_| crate::common::value_out_of_range_error())?;
                next.maximum_send_delay = Some(AuditSendDelay::new(seconds)?);
                self.0.change_configuration(next, source)
            }
            (PropertyIdentifier::SEND_NOW, PropertyValue::Boolean(value)) => {
                self.0.command_send_now(value, source)
            }
            _ => Err(crate::common::invalid_data_type_error()),
        }
    }
}

impl AuditReporterStatus {
    /// Current external capture evidence; internal command reset never changes it.
    #[doc(hidden)]
    pub fn capture_revision(&self) -> u64 {
        self.0.lock().unwrap().capture_revision
    }
    /// Monotonic configuration identity, independently of summary filtering.
    #[doc(hidden)]
    pub fn configuration_epoch(&self) -> u64 {
        self.0.lock().unwrap().configuration_epoch
    }
    /// Object-owned command readback; FALSE also represents terminal failure quiescence.
    #[doc(hidden)]
    pub fn send_now(&self) -> bool {
        self.0.lock().unwrap().send_now
    }
    /// Check command identity before preparing any external consequence.
    #[doc(hidden)]
    pub fn validate_send_now(&self, captured: bool) -> Result<(), Error> {
        let state = self.0.lock().unwrap();
        if state.command_revision == u64::MAX || (captured && state.capture_revision == u64::MAX) {
            return Err(Error::Encoding("audit command revision exhausted".into()));
        }
        Ok(())
    }
    /// Commit one validated command after resource preparation, before queue publication.
    #[doc(hidden)]
    pub fn commit_send_now(&self, value: bool, fence: u64, captured: bool) -> Result<(), Error> {
        let mut state = self.0.lock().unwrap();
        let revision = state
            .command_revision
            .checked_add(1)
            .ok_or_else(|| Error::Encoding("audit command revision exhausted".into()))?;
        let capture = if captured {
            state
                .capture_revision
                .checked_add(1)
                .ok_or_else(|| Error::Encoding("audit capture revision exhausted".into()))?
        } else {
            state.capture_revision
        };
        state.command_revision = revision;
        state.capture_revision = capture;
        state.send_now = value && fence != 0;
        if value {
            state.command_fence = fence;
        }
        Ok(())
    }
    /// Queue owns ordering: a later TRUE fence cannot be cleared by older work.
    #[doc(hidden)]
    pub fn settle_send_now(&self, first_unsent: Option<u64>) {
        let mut state = self.0.lock().unwrap();
        if first_unsent.is_none_or(|first| first > state.command_fence) {
            state.send_now = false;
        }
    }
}
