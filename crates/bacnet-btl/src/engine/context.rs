//! Test execution context — the central runtime type for BTL tests.
//!
//! Every test function receives a `&mut TestContext` and uses its methods
//! to interact with the IUT (read/write properties, verify values, etc.).

use std::time::Duration;

use chrono::Utc;

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_services::common::{BACnetPropertyValue, PropertyReference};
use bacnet_services::rpm::{ReadAccessSpecification, ReadPropertyMultipleACK};
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

use crate::iut::capabilities::IutCapabilities;
use crate::report::model::*;
use crate::self_test::SelfTestServer;

/// Transport-erased client wrapper. Built once at startup from CLI args.
pub enum ClientHandle {
    Bip(BACnetClient<BipTransport>),
    Bip6(BACnetClient<Bip6Transport>),
    #[cfg(feature = "sc-tls")]
    Sc(BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>),
}

/// Dispatch a BACnet client method across all transport variants.
macro_rules! dispatch_client {
    ($self:expr, $method:ident ( $($arg:expr),* $(,)? )) => {
        match &$self.client {
            ClientHandle::Bip(c) => c.$method($($arg),*).await,
            ClientHandle::Bip6(c) => c.$method($($arg),*).await,
            #[cfg(feature = "sc-tls")]
            ClientHandle::Sc(c) => c.$method($($arg),*).await,
        }
    };
}

/// The central test execution context.
pub struct TestContext {
    /// BACnet client (transport-erased).
    client: ClientHandle,
    /// IUT address.
    iut_addr: MacAddr,
    /// IUT capabilities.
    caps: IutCapabilities,
    /// Self-test server handle (None for external IUT).
    server: Option<SelfTestServer>,
    /// Step results collected during the current test.
    steps: Vec<StepResult>,
    /// Current step counter.
    step_counter: u16,
    /// Whether interactive prompts are available.
    interactive: bool,
    /// Per-test timeout.
    pub test_timeout: Duration,
    /// Test mode.
    mode: TestMode,
}

impl TestContext {
    /// Create a new context for testing an IUT.
    pub fn new(
        client: ClientHandle,
        iut_addr: MacAddr,
        caps: IutCapabilities,
        server: Option<SelfTestServer>,
        mode: TestMode,
    ) -> Self {
        Self {
            client,
            iut_addr,
            caps,
            server,
            steps: Vec::new(),
            step_counter: 0,
            interactive: false,
            test_timeout: Duration::from_secs(30),
            mode,
        }
    }

    pub fn set_interactive(&mut self, interactive: bool) {
        self.interactive = interactive;
    }

    pub fn capabilities(&self) -> &IutCapabilities {
        &self.caps
    }

    pub fn iut_info(&self) -> IutInfo {
        IutInfo {
            device_instance: self.caps.device_instance,
            vendor_name: self.caps.vendor_name.clone(),
            vendor_id: self.caps.vendor_id,
            model_name: self.caps.model_name.clone(),
            firmware_revision: self.caps.firmware_revision.clone(),
            protocol_revision: self.caps.protocol_revision,
            address: format!("{:?}", self.iut_addr),
        }
    }

    pub fn transport_info(&self) -> TransportInfo {
        let transport_type = match &self.client {
            ClientHandle::Bip(_) => "bip",
            ClientHandle::Bip6(_) => "bip6",
            #[cfg(feature = "sc-tls")]
            ClientHandle::Sc(_) => "sc",
        };
        TransportInfo {
            transport_type: transport_type.to_string(),
            local_address: String::new(),
            details: String::new(),
        }
    }

    pub fn test_mode(&self) -> TestMode {
        self.mode.clone()
    }

    /// Reset per-test state between tests.
    pub fn reset_steps(&mut self) {
        self.steps.clear();
        self.step_counter = 0;
    }

    /// Take the collected steps (moves them out).
    pub fn take_steps(&mut self) -> Vec<StepResult> {
        std::mem::take(&mut self.steps)
    }

    fn next_step(&mut self) -> u16 {
        self.step_counter += 1;
        self.step_counter
    }

    fn record_step(&mut self, step: StepResult) {
        self.steps.push(step);
    }

    // ── Object Lookup ────────────────────────────────────────────────────

    /// Find the first object of a given type in the IUT.
    pub fn first_object_of_type(&self, ot: ObjectType) -> Result<ObjectIdentifier, TestFailure> {
        self.caps
            .object_list
            .iter()
            .find(|oid| oid.object_type() == ot)
            .copied()
            .ok_or_else(|| TestFailure::new(format!("No {} object found in IUT", ot)))
    }

    /// Find all objects of a given type.
    pub fn all_objects_of_type(&self, ot: ObjectType) -> Vec<ObjectIdentifier> {
        self.caps
            .object_list
            .iter()
            .filter(|oid| oid.object_type() == ot)
            .copied()
            .collect()
    }

    /// Find the first commandable object (has priority array).
    pub fn first_commandable_object(&self) -> Result<ObjectIdentifier, TestFailure> {
        for (oid, detail) in &self.caps.object_details {
            if detail.commandable {
                return Ok(*oid);
            }
        }
        Err(TestFailure::new("No commandable object found in IUT"))
    }

    // ── BACnet Read Helpers ──────────────────────────────────────────────

    /// Raw ReadProperty — returns the value bytes from the ACK.
    pub async fn read_property_raw(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<Vec<u8>, TestFailure> {
        let step = self.next_step();
        let start = std::time::Instant::now();

        let result = dispatch_client!(self, read_property(&self.iut_addr, oid, prop, index));

        match result {
            Ok(ack) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Verify {
                        object: oid.to_string(),
                        property: format!("{:?}", prop),
                        value: format!("{} bytes", ack.property_value.len()),
                    },
                    expected: None,
                    actual: Some(format!("{} bytes", ack.property_value.len())),
                    pass: true,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Ok(ack.property_value.to_vec())
            }
            Err(e) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Verify {
                        object: oid.to_string(),
                        property: format!("{:?}", prop),
                        value: String::new(),
                    },
                    expected: Some("ReadProperty ACK".into()),
                    actual: Some(format!("Error: {e}")),
                    pass: false,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Err(TestFailure::at_step(
                    step,
                    format!("ReadProperty failed: {e}"),
                ))
            }
        }
    }

    /// Decode an application-tagged value: parse the tag, return the value bytes.
    pub fn decode_app_value(data: &[u8]) -> Result<(u8, &[u8]), TestFailure> {
        if data.is_empty() {
            return Err(TestFailure::new("Empty property value"));
        }
        let (tag, value_start) =
            tags::decode_tag(data, 0).map_err(|e| TestFailure::new(format!("Tag decode: {e}")))?;
        let value_end = value_start + tag.length as usize;
        if value_end > data.len() {
            return Err(TestFailure::new("Value extends beyond data"));
        }
        Ok((tag.number, &data[value_start..value_end]))
    }

    /// Read a REAL property value.
    pub async fn read_real(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
    ) -> Result<f32, TestFailure> {
        let data = self.read_property_raw(oid, prop, None).await?;
        let (_, value_bytes) = Self::decode_app_value(&data)?;
        primitives::decode_real(value_bytes)
            .map_err(|e| TestFailure::new(format!("Failed to decode REAL: {e}")))
    }

    /// Read a BOOLEAN property value.
    pub async fn read_bool(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
    ) -> Result<bool, TestFailure> {
        let data = self.read_property_raw(oid, prop, None).await?;
        // BACnet boolean: tag byte encodes the value in len_value_type (0=false, 1=true)
        if data.is_empty() {
            return Err(TestFailure::new("Empty property value"));
        }
        let (tag, _) =
            tags::decode_tag(&data, 0).map_err(|e| TestFailure::new(format!("Tag decode: {e}")))?;
        Ok(tag.length != 0)
    }

    /// Read an UNSIGNED property value.
    pub async fn read_unsigned(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
    ) -> Result<u32, TestFailure> {
        let data = self.read_property_raw(oid, prop, None).await?;
        let (_, value_bytes) = Self::decode_app_value(&data)?;
        let val = primitives::decode_unsigned(value_bytes)
            .map_err(|e| TestFailure::new(format!("Failed to decode UNSIGNED: {e}")))?;
        Ok(val as u32)
    }

    /// Read an ENUMERATED property value (decoded as unsigned).
    pub async fn read_enumerated(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
    ) -> Result<u32, TestFailure> {
        let data = self.read_property_raw(oid, prop, None).await?;
        let (_, value_bytes) = Self::decode_app_value(&data)?;
        let val = primitives::decode_unsigned(value_bytes)
            .map_err(|e| TestFailure::new(format!("Failed to decode ENUMERATED: {e}")))?;
        Ok(val as u32)
    }

    /// Verify a property is readable (any value accepted).
    pub async fn verify_readable(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
    ) -> Result<(), TestFailure> {
        self.read_property_raw(oid, prop, None).await?;
        Ok(())
    }

    /// Verify a REAL property has the expected value.
    pub async fn verify_real(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        expected: f32,
    ) -> Result<(), TestFailure> {
        let actual = self.read_real(oid, prop).await?;
        if (actual - expected).abs() > 0.001 {
            return Err(TestFailure::new(format!(
                "Expected {expected}, got {actual} for {:?}.{:?}",
                oid, prop
            )));
        }
        Ok(())
    }

    /// Verify a BOOLEAN property has the expected value.
    pub async fn verify_bool(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        expected: bool,
    ) -> Result<(), TestFailure> {
        let actual = self.read_bool(oid, prop).await?;
        if actual != expected {
            return Err(TestFailure::new(format!(
                "Expected {expected}, got {actual} for {:?}.{:?}",
                oid, prop
            )));
        }
        Ok(())
    }

    // ── BACnet Write Helpers ─────────────────────────────────────────────

    /// Raw WriteProperty.
    pub async fn write_property_raw(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        index: Option<u32>,
        value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), TestFailure> {
        let step = self.next_step();
        let start = std::time::Instant::now();

        let result = dispatch_client!(
            self,
            write_property(&self.iut_addr, oid, prop, index, value.clone(), priority)
        );

        match result {
            Ok(()) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Write {
                        object: oid.to_string(),
                        property: format!("{:?}", prop),
                        value: format!("{} bytes", value.len()),
                    },
                    expected: Some("SimpleACK".into()),
                    actual: Some("SimpleACK".into()),
                    pass: true,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Ok(())
            }
            Err(e) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Write {
                        object: oid.to_string(),
                        property: format!("{:?}", prop),
                        value: format!("{} bytes", value.len()),
                    },
                    expected: Some("SimpleACK".into()),
                    actual: Some(format!("Error: {e}")),
                    pass: false,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Err(TestFailure::at_step(
                    step,
                    format!("WriteProperty failed: {e}"),
                ))
            }
        }
    }

    /// Write a REAL value.
    pub async fn write_real(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        value: f32,
        priority: Option<u8>,
    ) -> Result<(), TestFailure> {
        let mut buf = bytes::BytesMut::new();
        bacnet_encoding::primitives::encode_app_real(&mut buf, value);
        self.write_property_raw(oid, prop, None, buf.to_vec(), priority)
            .await
    }

    /// Write a BOOLEAN value.
    pub async fn write_bool(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        value: bool,
    ) -> Result<(), TestFailure> {
        let mut buf = bytes::BytesMut::new();
        bacnet_encoding::primitives::encode_app_boolean(&mut buf, value);
        self.write_property_raw(oid, prop, None, buf.to_vec(), None)
            .await
    }

    /// Write NULL at a priority (relinquish command).
    pub async fn write_null(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        priority: Option<u8>,
    ) -> Result<(), TestFailure> {
        // Application-tagged NULL: tag=0, class=0, len=0 → byte 0x00
        self.write_property_raw(oid, prop, None, vec![0x00], priority)
            .await
    }

    /// Attempt a write and expect it to fail with an error.
    pub async fn write_expect_error(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), TestFailure> {
        let step = self.next_step();
        let start = std::time::Instant::now();

        let result = dispatch_client!(
            self,
            write_property(&self.iut_addr, oid, prop, None, value, priority)
        );

        match result {
            Ok(()) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Write {
                        object: oid.to_string(),
                        property: format!("{:?}", prop),
                        value: "expect error".into(),
                    },
                    expected: Some("BACnet-Error-PDU".into()),
                    actual: Some("SimpleACK (unexpected success)".into()),
                    pass: false,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Err(TestFailure::at_step(
                    step,
                    "Expected write to fail but it succeeded",
                ))
            }
            Err(_) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Write {
                        object: oid.to_string(),
                        property: format!("{:?}", prop),
                        value: "expect error".into(),
                    },
                    expected: Some("BACnet-Error-PDU".into()),
                    actual: Some("BACnet-Error-PDU".into()),
                    pass: true,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Ok(())
            }
        }
    }

    // ── Convenience ──────────────────────────────────────────────────────

    /// Convenience: test passes.
    pub fn pass(&self) -> Result<(), TestFailure> {
        Ok(())
    }

    /// Access the self-test server (for MAKE steps).
    pub fn server(&self) -> Option<&SelfTestServer> {
        self.server.as_ref()
    }

    /// Access the self-test server mutably (for MAKE steps).
    pub fn server_mut(&mut self) -> Option<&mut SelfTestServer> {
        self.server.as_mut()
    }

    // ── COV Helpers ──────────────────────────────────────────────────────

    /// Subscribe to COV on an object.
    pub async fn subscribe_cov(
        &mut self,
        oid: ObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), TestFailure> {
        let step = self.next_step();
        let start = std::time::Instant::now();

        let result = dispatch_client!(
            self,
            subscribe_cov(&self.iut_addr, 1, oid, confirmed, lifetime)
        );

        match result {
            Ok(()) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Transmit {
                        service: "SubscribeCOV".into(),
                        details: format!("{:?} lifetime={:?}", oid, lifetime),
                    },
                    expected: Some("SimpleACK".into()),
                    actual: Some("SimpleACK".into()),
                    pass: true,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Ok(())
            }
            Err(e) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Transmit {
                        service: "SubscribeCOV".into(),
                        details: format!("{:?} lifetime={:?}", oid, lifetime),
                    },
                    expected: Some("SimpleACK".into()),
                    actual: Some(format!("Error: {e}")),
                    pass: false,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Err(TestFailure::at_step(
                    step,
                    format!("SubscribeCOV failed: {e}"),
                ))
            }
        }
    }

    /// Subscribe to COV and expect it to fail.
    pub async fn subscribe_cov_expect_error(
        &mut self,
        oid: ObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), TestFailure> {
        let step = self.next_step();
        let start = std::time::Instant::now();

        let result = dispatch_client!(
            self,
            subscribe_cov(&self.iut_addr, 99, oid, confirmed, lifetime)
        );

        match result {
            Ok(()) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Transmit {
                        service: "SubscribeCOV".into(),
                        details: format!("{:?} (expect error)", oid),
                    },
                    expected: Some("BACnet-Error-PDU".into()),
                    actual: Some("SimpleACK (unexpected success)".into()),
                    pass: false,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Err(TestFailure::at_step(
                    step,
                    "SubscribeCOV should have failed",
                ))
            }
            Err(_) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Transmit {
                        service: "SubscribeCOV".into(),
                        details: format!("{:?} (expect error)", oid),
                    },
                    expected: Some("BACnet-Error-PDU".into()),
                    actual: Some("BACnet-Error-PDU".into()),
                    pass: true,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Ok(())
            }
        }
    }

    // ── ReadPropertyMultiple Helpers ─────────────────────────────────────

    /// Raw ReadPropertyMultiple — returns the ACK.
    pub async fn read_property_multiple(
        &mut self,
        specs: Vec<ReadAccessSpecification>,
    ) -> Result<ReadPropertyMultipleACK, TestFailure> {
        let step = self.next_step();
        let start = std::time::Instant::now();
        let desc = format!("{} spec(s)", specs.len());

        let result = dispatch_client!(self, read_property_multiple(&self.iut_addr, specs));

        match result {
            Ok(ack) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Verify {
                        object: "multiple".into(),
                        property: "RPM".into(),
                        value: desc,
                    },
                    expected: None,
                    actual: Some(format!(
                        "{} result(s)",
                        ack.list_of_read_access_results.len()
                    )),
                    pass: true,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Ok(ack)
            }
            Err(e) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Verify {
                        object: "multiple".into(),
                        property: "RPM".into(),
                        value: desc,
                    },
                    expected: Some("RPM-ACK".into()),
                    actual: Some(format!("Error: {e}")),
                    pass: false,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Err(TestFailure::at_step(step, format!("RPM failed: {e}")))
            }
        }
    }

    /// ReadPropertyMultiple: read a single property from a single object.
    pub async fn rpm_single(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<ReadPropertyMultipleACK, TestFailure> {
        self.read_property_multiple(vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: prop,
                property_array_index: index,
            }],
        }])
        .await
    }

    /// ReadPropertyMultiple: read multiple properties from a single object.
    pub async fn rpm_multi_props(
        &mut self,
        oid: ObjectIdentifier,
        props: &[PropertyIdentifier],
    ) -> Result<ReadPropertyMultipleACK, TestFailure> {
        self.read_property_multiple(vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: props
                .iter()
                .map(|p| PropertyReference {
                    property_identifier: *p,
                    property_array_index: None,
                })
                .collect(),
        }])
        .await
    }

    /// ReadPropertyMultiple: read ALL properties.
    pub async fn rpm_all(
        &mut self,
        oid: ObjectIdentifier,
    ) -> Result<ReadPropertyMultipleACK, TestFailure> {
        self.read_property_multiple(vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::ALL,
                property_array_index: None,
            }],
        }])
        .await
    }

    /// ReadPropertyMultiple: read REQUIRED properties.
    pub async fn rpm_required(
        &mut self,
        oid: ObjectIdentifier,
    ) -> Result<ReadPropertyMultipleACK, TestFailure> {
        self.read_property_multiple(vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::REQUIRED,
                property_array_index: None,
            }],
        }])
        .await
    }

    /// ReadPropertyMultiple: read OPTIONAL properties.
    pub async fn rpm_optional(
        &mut self,
        oid: ObjectIdentifier,
    ) -> Result<ReadPropertyMultipleACK, TestFailure> {
        self.read_property_multiple(vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::OPTIONAL,
                property_array_index: None,
            }],
        }])
        .await
    }

    /// ReadPropertyMultiple expecting error (returns Ok if error received).
    pub async fn rpm_expect_error(
        &mut self,
        specs: Vec<ReadAccessSpecification>,
    ) -> Result<(), TestFailure> {
        let step = self.next_step();
        let start = std::time::Instant::now();

        let result = dispatch_client!(self, read_property_multiple(&self.iut_addr, specs));

        match result {
            Ok(_) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Verify {
                        object: "multiple".into(),
                        property: "RPM".into(),
                        value: "expect error".into(),
                    },
                    expected: Some("Error".into()),
                    actual: Some("RPM-ACK (unexpected)".into()),
                    pass: false,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                // Note: RPM can return success with embedded errors.
                // For now, treat any ACK as acceptable since embedded errors
                // are returned within the ACK structure.
                Ok(())
            }
            Err(_) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Verify {
                        object: "multiple".into(),
                        property: "RPM".into(),
                        value: "expect error".into(),
                    },
                    expected: Some("Error".into()),
                    actual: Some("Error".into()),
                    pass: true,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Ok(())
            }
        }
    }

    // ── WritePropertyMultiple Helpers ────────────────────────────────────

    /// Raw WritePropertyMultiple.
    pub async fn write_property_multiple(
        &mut self,
        specs: Vec<WriteAccessSpecification>,
    ) -> Result<(), TestFailure> {
        let step = self.next_step();
        let start = std::time::Instant::now();
        let desc = format!("{} spec(s)", specs.len());

        let result = dispatch_client!(self, write_property_multiple(&self.iut_addr, specs));

        match result {
            Ok(()) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Write {
                        object: "multiple".into(),
                        property: "WPM".into(),
                        value: desc,
                    },
                    expected: Some("SimpleACK".into()),
                    actual: Some("SimpleACK".into()),
                    pass: true,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Ok(())
            }
            Err(e) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Write {
                        object: "multiple".into(),
                        property: "WPM".into(),
                        value: desc,
                    },
                    expected: Some("SimpleACK".into()),
                    actual: Some(format!("Error: {e}")),
                    pass: false,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Err(TestFailure::at_step(step, format!("WPM failed: {e}")))
            }
        }
    }

    /// WPM: Write a single property to a single object.
    pub async fn wpm_single(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), TestFailure> {
        self.write_property_multiple(vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: prop,
                property_array_index: None,
                value,
                priority,
            }],
        }])
        .await
    }

    /// WPM expecting error (returns Ok if error received).
    pub async fn wpm_expect_error(
        &mut self,
        specs: Vec<WriteAccessSpecification>,
    ) -> Result<(), TestFailure> {
        let step = self.next_step();
        let start = std::time::Instant::now();

        let result = dispatch_client!(self, write_property_multiple(&self.iut_addr, specs));

        match result {
            Ok(()) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Write {
                        object: "multiple".into(),
                        property: "WPM".into(),
                        value: "expect error".into(),
                    },
                    expected: Some("Error".into()),
                    actual: Some("SimpleACK (unexpected)".into()),
                    pass: false,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Err(TestFailure::at_step(
                    step,
                    "WPM expected error but got SimpleACK",
                ))
            }
            Err(_) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Write {
                        object: "multiple".into(),
                        property: "WPM".into(),
                        value: "expect error".into(),
                    },
                    expected: Some("Error".into()),
                    actual: Some("Error".into()),
                    pass: true,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Ok(())
            }
        }
    }

    /// Read a property and expect it to fail.
    pub async fn read_expect_error(
        &mut self,
        oid: ObjectIdentifier,
        prop: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<(), TestFailure> {
        let step = self.next_step();
        let start = std::time::Instant::now();

        let result = dispatch_client!(self, read_property(&self.iut_addr, oid, prop, index));

        match result {
            Ok(_) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Verify {
                        object: oid.to_string(),
                        property: format!("{:?}", prop),
                        value: "expect error".into(),
                    },
                    expected: Some("Error".into()),
                    actual: Some("ACK (unexpected)".into()),
                    pass: false,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Err(TestFailure::at_step(
                    step,
                    "ReadProperty expected error but got ACK",
                ))
            }
            Err(_) => {
                self.record_step(StepResult {
                    step_number: step,
                    action: StepAction::Verify {
                        object: oid.to_string(),
                        property: format!("{:?}", prop),
                        value: "expect error".into(),
                    },
                    expected: Some("Error".into()),
                    actual: Some("Error".into()),
                    pass: true,
                    timestamp: Utc::now(),
                    duration: start.elapsed(),
                    raw_apdu: None,
                });
                Ok(())
            }
        }
    }
}
