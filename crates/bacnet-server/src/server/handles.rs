use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Return the last bound local MAC snapshot, including after stop.
    /// This retained address does not assert that the transport is still active.
    pub fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    /// The transport the server's network layer runs on, for its
    /// transport-specific state: a B/IPv6 server's `local_address()`, which
    /// a peer turns into this server's VMAC with `resolve_virtual_address`,
    /// or a transport's counters. `None` once the server no longer holds its
    /// network layer.
    pub fn transport(&self) -> Option<&T> {
        self.network.as_deref().map(NetworkLayer::transport)
    }

    /// Get a reference to the shared object database.
    ///
    /// Objects read through this handle return raw standalone data. A built-in
    /// Device follows its declared service profile, with absent or empty COV lists.
    /// Served Device definitions and live lists belong to the executor: use
    /// [`read_local`](Self::read_local) for that view. Normal Device mutation and
    /// replacement cannot change the served execution profile.
    /// Install Device objects before startup: changing their membership through
    /// this handle does not rebind the discovery limiter's startup identity.
    ///
    /// A clone the application keeps can outlive the server's own handle, and
    /// whichever goes last drops every object, waiting for the saves durable
    /// objects have queued. In async code, let go of such a clone with
    /// [`drop_database_off_runtime`], so that wait runs on Tokio's blocking
    /// pool instead of a runtime worker (#1513).
    pub fn database(&self) -> &Arc<RwLock<ObjectDatabase>> {
        &self.db
    }

    /// Read one local property through the server's ReadProperty evaluator.
    ///
    /// Device wildcard reads target the configured identity; removal returns
    /// UNKNOWN_OBJECT even if another Device is present. The database's lowest
    /// Device instance still owns the live COV lists, independent of insertion
    /// order; other Device objects receive empty lists. Every served Device uses
    /// the executor's services, COV presence and property definitions, even after
    /// public profile mutation, replacement or custom object installation.
    ///
    /// This is the live local read boundary: it applies the same Device
    /// wildcard resolution, `UNKNOWN_OBJECT` and `PROPERTY_IS_NOT_AN_ARRAY`
    /// checks, and value source as network ReadProperty. The selected Device's
    /// `Active_COV_Subscriptions` (ordinary and single-property subscriptions)
    /// and `Active_COV_Multiple_Subscriptions` (SubscribeCOVPropertyMultiple
    /// contexts) are projected from the COV subscription table at one sampled
    /// instant, returned as their encoded `BACnetLIST of BACnetCOVSubscription`
    /// and `BACnetLIST of BACnetCOVMultipleSubscription`
    /// (`PropertyValue::ApplicationData`). After [`stop`](Self::stop) the
    /// server services no subscriptions, so both properties read as empty lists.
    ///
    /// The selected Device's `Device_Address_Binding` lists the server's device
    /// bindings at the time of the read (#1369): each configured
    /// [`DeviceBinding`] and each device whose I-Am was heard in the last ten
    /// minutes, in Device instance order, as a `PropertyValue::List` whose
    /// items are encoded BACnetAddressBindings
    /// (`PropertyValue::ApplicationData`). A device on this network has
    /// network number 0. The bindings outlast `stop`, so they still read then.
    ///
    /// A Group's `Present_Value` is rebuilt from its members, whose rows count
    /// against the ReadPropertyMultiple work limit
    /// ([`ReadPropertyMultipleBudget::max_result_elements`](crate::server::ReadPropertyMultipleBudget))
    /// as for network ReadProperty; past it the read fails with
    /// [`Error::abort`] carrying OUT_OF_RESOURCES.
    pub async fn read_local(
        &self,
        oid: &ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        let db = self.db.read().await;
        let view = crate::device_view::DeviceReadContext::new(
            &db,
            crate::device_view::DeviceExecution::FullServer,
        )
        .with_identity(Some(&self.config.device))
        .with_work_limit(
            self.config
                .read_property_multiple_budget
                .max_result_elements,
        );
        let lookup_oid = handlers::resolve_read_target(
            &db,
            oid,
            self.config.registered_network_port,
            view.device_oid(),
        );
        let plan =
            handlers::plan_read_property(&db, Some(&view), lookup_oid, property, array_index)
                .map_err(|failure| match failure {
                    handlers::ReadFailure::Service(error) => error,
                    handlers::ReadFailure::Work | handlers::ReadFailure::Bytes => {
                        Error::abort(AbortReason::OUT_OF_RESOURCES)
                    }
                })?;
        use super::requests::confirmed_response::{active_cov_snapshot, address_bindings};
        let live = match plan.live_cov(&db) {
            Some(selection) if self.dispatch_task.is_none() => {
                let bindings = address_bindings(&self.device_bindings, selection).await;
                Some(
                    crate::cov::active::LiveDeviceCov::stopped(selection)
                        .with_address_bindings(selection, bindings),
                )
            }
            Some(selection) => {
                let tables = super::requests::confirmed_response::LiveTables {
                    cov: &self.cov_table,
                    bindings: &self.device_bindings,
                };
                Some(active_cov_snapshot(&db, tables, selection).await)
            }
            None => None,
        };
        let view = view.with_live(live.as_ref());
        handlers::read_property_value(&db, Some(&view), plan)
    }

    /// Write one local property from its encoded value: the octets a network
    /// WriteProperty would carry for it.
    ///
    /// The octets take the steps the WriteProperty handler takes before an
    /// object sees a value. An array index is refused as it is over the
    /// network (UNKNOWN_PROPERTY for a property the object doesn't hold,
    /// PROPERTY_IS_NOT_AN_ARRAY for one that isn't an array), and the octets
    /// are decoded with that handler's per-property framing, which hands a
    /// Schedule's Effective_Period or a Staging's Stages to the object in the
    /// form it decodes. The value then takes the [`write_local`](Self::write_local)
    /// path, with its COV, event and audit work. A value read with
    /// [`read_local`](Self::read_local) and encoded therefore writes back, and
    /// any value a network client could write is accepted here.
    ///
    /// Like `write_local`, it must be awaited inside a Tokio runtime, failing
    /// before anything is written outside one, and a caller dropped once the
    /// write has committed skips none of the work the write owes (#1367).
    pub async fn write_local_encoded(
        &self,
        oid: &ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &[u8],
        priority: Option<u8>,
        source: crate::LocalCommandSource,
    ) -> Result<(), Error> {
        self.active_network()?;
        let value = {
            let db = self.db.read().await;
            let object = db.get(oid).ok_or(Error::protocol(
                ErrorClass::OBJECT,
                ErrorCode::UNKNOWN_OBJECT,
            ))?;
            handlers::gate_and_decode_write(object, property, array_index, value)?
        };
        self.write_local(oid, property, array_index, value, priority, source)
            .await
    }

    /// Create a cloneable handle for unsolicited I-Am announcements.
    pub fn i_am_broadcaster(&self) -> IAmBroadcaster<T> {
        IAmBroadcaster {
            state: Arc::downgrade(&self.broadcaster),
        }
    }

    /// Get the communication state per DeviceCommunicationControl.
    ///
    /// The server refuses the deprecated DISABLE, so this is
    /// [`DccState::Enable`] or [`DccState::DisableInitiation`].
    pub fn comm_state(&self) -> DccState {
        self.comm_state.get()
    }

    /// Generate PICS from the database and the server's effective Device execution view.
    /// Standalone [`PicsGenerator`](crate::pics::PicsGenerator) keeps raw-object semantics.
    ///
    /// The caller must supply a [`PicsConfig`](crate::pics::PicsConfig) for fields not available from the server
    /// (device profile, data links, network roles, etc.). The vendor and model names, the firmware and
    /// application versions, and the protocol version and revision come from the served Device object,
    /// so the PICS can't contradict what the device reports.
    pub async fn generate_pics(&self, pics_config: &crate::pics::PicsConfig) -> crate::pics::Pics {
        let db = self.db.read().await;
        crate::pics::PicsGenerator::new(&db, &self.config, pics_config)
            .for_server()
            .generate()
    }
}

impl<T: TransportPort + bacnet_transport::bip::AsBip + 'static> BACnetServer<T> {
    /// How this server's foreign-device registration is going (#938):
    /// whether it holds a live lease, the time left on it, and the last
    /// attempt's result and outcome, a BBMD's refusal included. It reads
    /// [`BipTransport::bvlc_client_snapshot`] and sends nothing. Fails on a
    /// transport that isn't B/IP, and once the server has stopped.
    pub fn foreign_registration(
        &self,
    ) -> Result<bacnet_transport::bip::ForeignRegistrationSnapshot, Error> {
        let transport = self.transport().ok_or_else(broadcaster::stopped)?;
        Ok(transport
            .as_bip()?
            .bvlc_client_snapshot()
            .foreign_registration)
    }
}

#[cfg(test)]
#[path = "local_encoded_writes_tests.rs"]
mod local_encoded_writes_tests;
