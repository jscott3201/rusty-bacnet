//! Java/Kotlin BACnetServer — UniFFI wrapper around the Rust BACnetServer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;

use bacnet_objects::access_control::*;
use bacnet_objects::accumulator::*;
use bacnet_objects::analog::*;
use bacnet_objects::audit::*;
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::binary::*;
use bacnet_objects::command::CommandObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::elevator::*;
use bacnet_objects::event_enrollment::*;
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::file::FileObject;
use bacnet_objects::forwarder::NotificationForwarderObject;
use bacnet_objects::group::*;
use bacnet_objects::life_safety::*;
use bacnet_objects::lighting::*;
use bacnet_objects::load_control::LoadControlObject;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::multistate::*;
use bacnet_objects::network_port::NetworkPortObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::program::ProgramObject;
use bacnet_objects::schedule::*;
use bacnet_objects::staging::StagingObject;
use bacnet_objects::timer::TimerObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::*;
use bacnet_objects::value_types::*;
use bacnet_server::server;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::mstp::NoSerial;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::errors::BacnetError;
use crate::transport::build_transport;
use crate::types::*;

/// BACnet server that hosts objects and responds to network requests.
#[derive(uniffi::Object)]
pub struct BacnetServer {
    inner: Arc<Mutex<Option<server::BACnetServer<AnyTransport<NoSerial>>>>>,
    device_instance: u32,
    device_name: String,
    config: TransportConfig,
    started: Arc<AtomicBool>,
    pending_objects: std::sync::Mutex<Vec<Box<dyn BACnetObject + Send>>>,
}

impl BacnetServer {
    fn push_pending(&self, obj: Box<dyn BACnetObject + Send>) -> Result<(), BacnetError> {
        let mut guard = self
            .pending_objects
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.started.load(Ordering::Acquire) {
            return Err(BacnetError::InvalidArgument {
                msg: "cannot add objects after start()".into(),
            });
        }
        guard.push(obj);
        Ok(())
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl BacnetServer {
    #[uniffi::constructor]
    pub fn new(device_instance: u32, device_name: String, config: TransportConfig) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Mutex::new(None)),
            device_instance,
            device_name,
            config,
            started: Arc::new(AtomicBool::new(false)),
            pending_objects: std::sync::Mutex::new(Vec::new()),
        })
    }

    // ---- Analog types ----

    pub fn add_analog_input(
        &self,
        instance: u32,
        name: String,
        units: u32,
        present_value: f32,
    ) -> Result<(), BacnetError> {
        let mut ai = AnalogInputObject::new(instance, &name, units).map_err(BacnetError::from)?;
        ai.set_present_value(present_value);
        self.push_pending(Box::new(ai))
    }

    pub fn add_analog_output(
        &self,
        instance: u32,
        name: String,
        units: u32,
    ) -> Result<(), BacnetError> {
        let obj = AnalogOutputObject::new(instance, &name, units).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_analog_value(
        &self,
        instance: u32,
        name: String,
        units: u32,
    ) -> Result<(), BacnetError> {
        let obj = AnalogValueObject::new(instance, &name, units).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Binary types ----

    pub fn add_binary_input(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = BinaryInputObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_binary_output(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = BinaryOutputObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_binary_value(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = BinaryValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Multi-state types ----

    pub fn add_multistate_input(
        &self,
        instance: u32,
        name: String,
        number_of_states: u32,
    ) -> Result<(), BacnetError> {
        let obj = MultiStateInputObject::new(instance, &name, number_of_states)
            .map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_multistate_output(
        &self,
        instance: u32,
        name: String,
        number_of_states: u32,
    ) -> Result<(), BacnetError> {
        let obj = MultiStateOutputObject::new(instance, &name, number_of_states)
            .map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_multistate_value(
        &self,
        instance: u32,
        name: String,
        number_of_states: u32,
    ) -> Result<(), BacnetError> {
        let obj = MultiStateValueObject::new(instance, &name, number_of_states)
            .map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Value types ----

    pub fn add_integer_value(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = IntegerValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_positive_integer_value(
        &self,
        instance: u32,
        name: String,
    ) -> Result<(), BacnetError> {
        let obj = PositiveIntegerValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_large_analog_value(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = LargeAnalogValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_character_string_value(
        &self,
        instance: u32,
        name: String,
    ) -> Result<(), BacnetError> {
        let obj = CharacterStringValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_octet_string_value(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = OctetStringValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_bit_string_value(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = BitStringValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_date_value(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = DateValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_time_value(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = TimeValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_date_time_value(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = DateTimeValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_date_pattern_value(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = DatePatternValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_time_pattern_value(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = TimePatternValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_date_time_pattern_value(
        &self,
        instance: u32,
        name: String,
    ) -> Result<(), BacnetError> {
        let obj = DateTimePatternValueObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Scheduling & logs ----

    pub fn add_calendar(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = CalendarObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_schedule(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj =
            ScheduleObject::new(instance, &name, PropertyValue::Null).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_notification_class(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = NotificationClass::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_trend_log(
        &self,
        instance: u32,
        name: String,
        buffer_size: u32,
    ) -> Result<(), BacnetError> {
        let obj = TrendLogObject::new(instance, &name, buffer_size).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_trend_log_multiple(
        &self,
        instance: u32,
        name: String,
        buffer_size: u32,
    ) -> Result<(), BacnetError> {
        let obj =
            TrendLogMultipleObject::new(instance, &name, buffer_size).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_event_log(
        &self,
        instance: u32,
        name: String,
        buffer_size: u32,
    ) -> Result<(), BacnetError> {
        let obj = EventLogObject::new(instance, &name, buffer_size).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_audit_log(
        &self,
        instance: u32,
        name: String,
        buffer_size: u32,
    ) -> Result<(), BacnetError> {
        let obj = AuditLogObject::new(instance, &name, buffer_size).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_audit_reporter(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = AuditReporterObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Control & specialized ----

    pub fn add_loop(&self, instance: u32, name: String, units: u32) -> Result<(), BacnetError> {
        let obj = LoopObject::new(instance, &name, units).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_command(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = CommandObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_timer(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = TimerObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_load_control(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = LoadControlObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_program(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = ProgramObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Lighting ----

    pub fn add_lighting_output(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = LightingOutputObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_binary_lighting_output(
        &self,
        instance: u32,
        name: String,
    ) -> Result<(), BacnetError> {
        let obj = BinaryLightingOutputObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_channel(
        &self,
        instance: u32,
        name: String,
        channel_number: u32,
    ) -> Result<(), BacnetError> {
        let obj = ChannelObject::new(instance, &name, channel_number).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Life safety ----

    pub fn add_life_safety_point(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = LifeSafetyPointObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_life_safety_zone(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = LifeSafetyZoneObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Access control ----

    pub fn add_access_door(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = AccessDoorObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_access_credential(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = AccessCredentialObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_access_point(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = AccessPointObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_access_rights(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = AccessRightsObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_access_user(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = AccessUserObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_access_zone(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = AccessZoneObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_credential_data_input(
        &self,
        instance: u32,
        name: String,
    ) -> Result<(), BacnetError> {
        let obj = CredentialDataInputObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Grouping ----

    pub fn add_group(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = GroupObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_global_group(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = GlobalGroupObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_structured_view(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = StructuredViewObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_notification_forwarder(
        &self,
        instance: u32,
        name: String,
    ) -> Result<(), BacnetError> {
        let obj = NotificationForwarderObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_alert_enrollment(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = AlertEnrollmentObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Counters & conversion ----

    pub fn add_accumulator(
        &self,
        instance: u32,
        name: String,
        units: u32,
    ) -> Result<(), BacnetError> {
        let obj = AccumulatorObject::new(instance, &name, units).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_pulse_converter(
        &self,
        instance: u32,
        name: String,
        units: u32,
    ) -> Result<(), BacnetError> {
        let obj = PulseConverterObject::new(instance, &name, units).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Building systems ----

    pub fn add_elevator_group(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = ElevatorGroupObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_escalator(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = EscalatorObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_lift(
        &self,
        instance: u32,
        name: String,
        num_floors: u32,
    ) -> Result<(), BacnetError> {
        let obj =
            LiftObject::new(instance, &name, num_floors as usize).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_staging(
        &self,
        instance: u32,
        name: String,
        num_stages: u32,
    ) -> Result<(), BacnetError> {
        let obj =
            StagingObject::new(instance, &name, num_stages as usize).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- File & network ----

    pub fn add_file(
        &self,
        instance: u32,
        name: String,
        file_type: String,
    ) -> Result<(), BacnetError> {
        let obj = FileObject::new(instance, &name, &file_type).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    pub fn add_network_port(
        &self,
        instance: u32,
        name: String,
        network_type: u32,
    ) -> Result<(), BacnetError> {
        let obj =
            NetworkPortObject::new(instance, &name, network_type).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Events ----

    pub fn add_event_enrollment(
        &self,
        instance: u32,
        name: String,
        event_type: u32,
    ) -> Result<(), BacnetError> {
        let obj =
            EventEnrollmentObject::new(instance, &name, event_type).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Other ----

    pub fn add_averaging(&self, instance: u32, name: String) -> Result<(), BacnetError> {
        let obj = AveragingObject::new(instance, &name).map_err(BacnetError::from)?;
        self.push_pending(Box::new(obj))
    }

    // ---- Runtime ----

    /// Start the server. Objects can no longer be added after this.
    pub async fn start(&self) -> Result<(), BacnetError> {
        if self.started.load(Ordering::Acquire) {
            return Err(BacnetError::InvalidArgument {
                msg: "server already started".into(),
            });
        }

        let transport = build_transport(&self.config)?;

        let mut device = DeviceObject::new(DeviceConfig {
            instance: self.device_instance,
            name: self.device_name.clone(),
            vendor_name: "Rusty BACnet".into(),
            vendor_id: 555,
            ..Default::default()
        })
        .map_err(BacnetError::from)?;

        let mut db = ObjectDatabase::new();

        // Move pending objects into the database
        let pending = {
            let mut guard = self
                .pending_objects
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        };

        let dev_oid = device.object_identifier();
        let mut object_list = vec![dev_oid];
        for obj in pending {
            object_list.push(obj.object_identifier());
            db.add(obj).map_err(|e| BacnetError::InvalidArgument {
                msg: format!("duplicate object: {e}"),
            })?;
        }

        device.set_object_list(object_list);
        db.add(Box::new(device))
            .map_err(|e| BacnetError::InvalidArgument {
                msg: format!("failed to add device: {e}"),
            })?;

        let srv = server::BACnetServer::generic_builder()
            .database(db)
            .transport(transport)
            .build()
            .await
            .map_err(BacnetError::from)?;

        self.started.store(true, Ordering::Release);
        *self.inner.lock().await = Some(srv);
        Ok(())
    }

    /// Stop the server.
    pub async fn stop(&self) -> Result<(), BacnetError> {
        let mut guard = self.inner.lock().await;
        if let Some(mut srv) = guard.take() {
            srv.stop().await.map_err(BacnetError::from)?;
        }
        self.started.store(false, Ordering::Release);
        Ok(())
    }

    /// Read a property from a local object (post-start).
    pub async fn read_property(
        &self,
        object_type: u32,
        object_instance: u32,
        property_id: u32,
        array_index: Option<u32>,
    ) -> Result<BacnetPropertyValue, BacnetError> {
        let db_arc = {
            let guard = self.inner.lock().await;
            let srv = guard.as_ref().ok_or(BacnetError::NotStarted)?;
            srv.database().clone()
        };
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), object_instance)
            .map_err(BacnetError::from)?;
        let pid = PropertyIdentifier::from_raw(property_id);

        let db = db_arc.read().await;
        let obj = db.get(&oid).ok_or(BacnetError::InvalidArgument {
            msg: format!("object not found: {oid}"),
        })?;
        let value = obj
            .read_property(pid, array_index)
            .map_err(BacnetError::from)?;

        Ok(crate::client::property_value_to_java(&value))
    }

    /// Write a property on a local object (post-start).
    pub async fn write_property_local(
        &self,
        object_type: u32,
        object_instance: u32,
        property_id: u32,
        value: BacnetPropertyValue,
        priority: Option<u8>,
        array_index: Option<u32>,
    ) -> Result<(), BacnetError> {
        let db_arc = {
            let guard = self.inner.lock().await;
            let srv = guard.as_ref().ok_or(BacnetError::NotStarted)?;
            srv.database().clone()
        };
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), object_instance)
            .map_err(BacnetError::from)?;
        let pid = PropertyIdentifier::from_raw(property_id);
        let prop_value = crate::client::java_to_property_value(&value)?;

        let mut db = db_arc.write().await;
        let obj = db.get_mut(&oid).ok_or(BacnetError::InvalidArgument {
            msg: format!("object not found: {oid}"),
        })?;
        obj.write_property(pid, array_index, prop_value, priority)
            .map_err(BacnetError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_construction() {
        let cfg = TransportConfig::Bip {
            address: "0.0.0.0".into(),
            port: 47808,
            broadcast_address: "255.255.255.255".into(),
        };
        let srv = BacnetServer::new(1234, "Test Device".into(), cfg);
        assert!(!srv.started.load(Ordering::Relaxed));
    }

    #[test]
    fn test_add_objects_before_start() {
        let cfg = TransportConfig::Bip {
            address: "0.0.0.0".into(),
            port: 47808,
            broadcast_address: "255.255.255.255".into(),
        };
        let srv = BacnetServer::new(1234, "Test Device".into(), cfg);
        assert!(srv.add_analog_input(1, "Temp".into(), 62, 72.5).is_ok());
        assert!(srv.add_binary_value(1, "Alarm".into()).is_ok());
        assert!(srv.add_multistate_input(1, "Mode".into(), 3).is_ok());
    }

    #[test]
    fn test_add_rejects_after_started_flag() {
        let cfg = TransportConfig::Bip {
            address: "0.0.0.0".into(),
            port: 47808,
            broadcast_address: "255.255.255.255".into(),
        };
        let srv = BacnetServer::new(1234, "Test Device".into(), cfg);
        srv.started.store(true, Ordering::Release);
        assert!(srv.add_binary_value(1, "Test".into()).is_err());
    }
}
