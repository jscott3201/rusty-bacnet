//! Device-owned recipient state and the synchronous runtime mutation capability.
use super::*;
use alloc::sync::Weak;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};

/// Actual remote requester context, supplied only after service authorization.
#[doc(hidden)]
#[derive(Clone)]
pub struct AuditWriteSource {
    pub device: BACnetRecipient,
    pub invoke_id: u8,
}

/// Installed target runtime. Implementations must prepare both deliveries before
/// changing `current`, then commit and register their owned worker atomically.
#[doc(hidden)]
pub trait AuditRecipientChangeSink: Send + Sync {
    fn change(
        &self,
        current: &mut BACnetRecipient,
        new: BACnetRecipient,
        source: Option<&AuditWriteSource>,
        clock: Option<ClockFrame>,
    ) -> Result<(), Error>;
    fn is_active(&self) -> bool;
}

#[derive(Default)]
pub(super) struct RecipientState {
    value: Option<BACnetRecipient>,
    sink: Option<Weak<dyn AuditRecipientChangeSink>>,
}

fn property_error(code: ErrorCode) -> Error {
    Error::protocol(ErrorClass::PROPERTY, code)
}

/// A scoped operation capability for the built-in Device. It deliberately does
/// not expose a mutable Device reference or permit replacing its installed state.
#[doc(hidden)]
pub struct DeviceAuthority<'a>(pub(super) &'a mut DeviceObject);

impl DeviceAuthority<'_> {
    pub fn object_identifier(&self) -> ObjectIdentifier {
        self.0.oid
    }
    pub fn set_services_supported(&mut self, services: &[ServiceSupported]) {
        self.0.set_services_supported(services);
    }
    pub fn set_object_list(&mut self, oids: Vec<ObjectIdentifier>) {
        self.0.set_object_list(oids);
    }
    pub fn set_system_status(&mut self, status: DeviceStatus) {
        self.0.set_system_status(status);
    }
    pub fn set_database_revision(&mut self, revision: u32) {
        self.0.set_database_revision(revision);
    }
    pub fn set_backup_and_restore_state(&mut self, state: BackupAndRestoreState) {
        self.0.set_backup_and_restore_state(state);
    }
    pub fn set_last_restore_time(&mut self, time: BACnetTimeStamp) {
        self.0.set_last_restore_time(time);
    }
    pub fn set_configuration_files(&mut self, files: Vec<ObjectIdentifier>) {
        self.0.set_configuration_files(files);
    }
    pub fn write_property(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.0.write_property(property, index, value, priority)
    }
    pub fn provision_audit_recipient(&mut self, recipient: BACnetRecipient) -> Result<(), Error> {
        self.0.provision_audit_recipient(recipient)
    }
    pub fn provisioned_audit_recipient(&self) -> Option<&BACnetRecipient> {
        self.0.recipient.value.as_ref()
    }
    pub fn validate_audit_recipient_installation(&self) -> Result<(), Error> {
        if self.0.recipient.value.is_none()
            || self
                .0
                .recipient
                .sink
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some()
        {
            return Err(property_error(ErrorCode::WRITE_ACCESS_DENIED));
        }
        Ok(())
    }
    pub fn install_audit_recipient(
        &mut self,
        sink: &Arc<dyn AuditRecipientChangeSink>,
    ) -> Result<(), Error> {
        self.validate_audit_recipient_installation()?;
        self.0.recipient.sink = Some(Arc::downgrade(sink));
        Ok(())
    }
    pub fn uninstall_audit_recipient(&mut self, sink: &Arc<dyn AuditRecipientChangeSink>) {
        if !sink.is_active()
            && self
                .0
                .recipient
                .sink
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|current| Arc::ptr_eq(&current, sink))
        {
            self.0.recipient.sink = None;
        }
    }
    pub fn write_audit_recipient(
        &mut self,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        source: Option<&AuditWriteSource>,
    ) -> Result<(), Error> {
        self.0.write_audit_recipient(index, value, priority, source)
    }
}

impl DeviceObject {
    /// Provision the initial target Audit recipient before installing a server
    /// runtime. This alone does not expose the network property. Active changes
    /// must use WriteProperty so both old and new recipients are notified.
    pub fn provision_audit_recipient(&mut self, recipient: BACnetRecipient) -> Result<(), Error> {
        validate_recipient(&recipient)?;
        if self
            .recipient
            .sink
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some()
        {
            return Err(property_error(ErrorCode::WRITE_ACCESS_DENIED));
        }
        self.recipient.value = Some(recipient);
        Ok(())
    }

    pub(super) fn audit_recipient_present(&self) -> bool {
        self.recipient.value.is_some()
            && self
                .recipient
                .sink
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some()
    }

    pub(super) fn read_audit_recipient(&self, index: Option<u32>) -> Result<PropertyValue, Error> {
        if !self.audit_recipient_present() {
            return Err(property_error(ErrorCode::UNKNOWN_PROPERTY));
        }
        if index.is_some() {
            return Err(property_error(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY));
        }
        let mut bytes = bytes::BytesMut::new();
        bacnet_encoding::constructed::encode_recipient(
            &mut bytes,
            self.recipient.value.as_ref().expect("present"),
        )?;
        Ok(PropertyValue::ApplicationData(bytes.to_vec()))
    }

    pub(super) fn write_audit_recipient(
        &mut self,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        source: Option<&AuditWriteSource>,
    ) -> Result<(), Error> {
        let sink = self
            .recipient
            .sink
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or_else(|| property_error(ErrorCode::UNKNOWN_PROPERTY))?;
        if !sink.is_active() {
            return Err(property_error(ErrorCode::WRITE_ACCESS_DENIED));
        }
        if index.is_some() {
            return Err(property_error(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY));
        }
        if priority.is_some_and(|p| !(1..=16).contains(&p)) {
            return Err(Error::protocol(
                ErrorClass::SERVICES,
                ErrorCode::PARAMETER_OUT_OF_RANGE,
            ));
        }
        if value == PropertyValue::Null {
            return Ok(());
        }
        let PropertyValue::ApplicationData(bytes) = value else {
            return Err(property_error(ErrorCode::INVALID_DATA_TYPE));
        };
        let (new, end) = bacnet_encoding::constructed::decode_recipient(&bytes, 0)
            .map_err(|_| property_error(ErrorCode::INVALID_DATA_ENCODING))?;
        if end != bytes.len() {
            return Err(property_error(ErrorCode::INVALID_DATA_ENCODING));
        }
        validate_recipient(&new)?;
        let clock = self.clock_frame();
        let current = self.recipient.value.as_mut().expect("installed recipient");
        if *current == new {
            return Ok(());
        }
        sink.change(current, new, source, clock)
    }
}

/// A recipient the property can hold: a concrete Device, or an address whose
/// MAC is within the configured-recipient bound. Provisioning checks the MAC
/// here because nothing decodes it on the way in (#1124); a write already has.
fn validate_recipient(recipient: &BACnetRecipient) -> Result<(), Error> {
    match recipient {
        BACnetRecipient::Device(oid)
            if oid.object_type() != ObjectType::DEVICE
                || oid.instance_number() == ObjectIdentifier::MAX_INSTANCE =>
        {
            Err(property_error(ErrorCode::INVALID_DATA_ENCODING))
        }
        BACnetRecipient::Address(address)
            if address.mac_address.len() > BACnetAddress::MAX_MAC_LEN =>
        {
            Err(property_error(ErrorCode::INVALID_DATA_ENCODING))
        }
        _ => Ok(()),
    }
}
