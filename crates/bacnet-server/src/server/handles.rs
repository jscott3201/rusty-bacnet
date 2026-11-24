use super::*;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard};

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

    /// The pool the server's confirmed notifications take their invoke IDs
    /// and workers from: on a node's seat, the node's source Audit sends
    /// through it too, so the server completes and joins those records
    /// (#2617).
    #[doc(hidden)]
    pub fn notification_pool(&self) -> &Arc<NotificationTransactions> {
        &self.notification_transactions
    }

    /// Read one local property through the server's ReadProperty evaluator.
    ///
    /// Device wildcard reads target the configured identity; removal returns
    /// UNKNOWN_OBJECT even if another Device is present. The identity's Device
    /// owns the live COV lists, even beside a lower-instance Device added later
    /// (#2227); other Device objects receive empty lists. Every served Device uses
    /// the executor's services, COV presence and property definitions, even after
    /// public profile mutation, replacement or custom object installation.
    ///
    /// This is the live local read boundary: it applies the same Device
    /// wildcard resolution, `UNKNOWN_OBJECT` and `PROPERTY_IS_NOT_AN_ARRAY`
    /// checks, and value source as network ReadProperty. The identity Device's
    /// `Active_COV_Subscriptions` (ordinary and single-property subscriptions)
    /// and `Active_COV_Multiple_Subscriptions` (SubscribeCOVPropertyMultiple
    /// contexts) are projected from the COV subscription table at one sampled
    /// instant, returned as their encoded `BACnetLIST of BACnetCOVSubscription`
    /// and `BACnetLIST of BACnetCOVMultipleSubscription`
    /// (`PropertyValue::ApplicationData`). After [`stop`](Self::stop) the
    /// server services no subscriptions, so both properties read as empty lists.
    ///
    /// The identity Device's `Device_Address_Binding` lists the server's device
    /// bindings at the time of the read (#1369): each configured
    /// [`DeviceBinding`] and each device whose I-Am was heard in the last ten
    /// minutes, in Device instance order, as a `PropertyValue::List` whose
    /// items are encoded BACnetAddressBindings
    /// (`PropertyValue::ApplicationData`). A device on this network has
    /// network number 0. The bindings outlast `stop`, so they still read then.
    ///
    /// A Group's `Present_Value` is rebuilt from its members, whose rows count
    /// against the ReadPropertyMultiple work limit
    /// ([`ReadPropertyMultipleBudget::max_result_elements`](bacnet_device_core::read::ReadPropertyMultipleBudget))
    /// as for network ReadProperty; past it the read fails with
    /// [`Error::abort`] carrying OUT_OF_RESOURCES.
    ///
    /// The read never waits for a lock in its caller's task (#2772): with
    /// the database or a table it reads busy, a task of the server's waits
    /// and reads, and the call returns what that task read, so a caller that
    /// polls the call once and then leaves it unpolled holds up none of the
    /// requests the server serves. [`write_local`](Self::write_local)'s Cancellation section
    /// describes this for every local call, and what a dropped call does.
    pub async fn read_local(
        &self,
        oid: &ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        let read = self.local_read().read(*oid, property, array_index);
        match self.request_tasks.for_caller(read, |value| value).await {
            Some(value) => value,
            // Its runtime dropped the task before it read: read here.
            None => self.local_read().read(*oid, property, array_index).await,
        }
    }

    /// The handles [`read_local`](Self::read_local) reads with.
    fn local_read(&self) -> LocalRead {
        LocalRead {
            db: Arc::clone(&self.db),
            config: Arc::clone(&self.config),
            execution: self.execution,
            cov_table: Arc::clone(&self.cov_table),
            device_bindings: Arc::clone(&self.device_bindings),
            stopped: self.dispatch_task.is_none(),
        }
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
    /// write has committed skips none of the work the write owes (#1367). A
    /// call dropped part way is as
    /// [`write_local`](Self::write_local)'s Cancellation section describes (#2772).
    pub async fn write_local_encoded(
        &self,
        oid: &ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &[u8],
        priority: Option<u8>,
        source: bacnet_device_core::write::local::LocalCommandSource,
    ) -> Result<(), Error> {
        self.active_network()?;
        let (target, encoded) = (*oid, value.to_vec());
        let decode = move |db: OwnedRwLockReadGuard<ObjectDatabase>| {
            decode_local(&db, &target, property, array_index, &encoded)
        };
        let decoded = self
            .request_tasks
            .for_caller(Arc::clone(&self.db).read_owned(), decode)
            .await;
        let value = match decoded {
            Some(value) => value,
            // Its runtime dropped the task before it decoded: decode here.
            None => decode_local(&*self.db.read().await, oid, property, array_index, value),
        }?;
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
        let (config, execution) = (Arc::clone(&self.config), self.execution);
        let owned = pics_config.clone();
        let generate = move |db: OwnedRwLockReadGuard<ObjectDatabase>| {
            crate::pics::PicsGenerator::new(&db, &config, &owned)
                .for_server(execution)
                .generate()
        };
        // Never waited for in the caller's task (#2772).
        let generated = self
            .request_tasks
            .for_caller(Arc::clone(&self.db).read_owned(), generate)
            .await;
        match generated {
            Some(pics) => pics,
            // Its runtime dropped the task before it read: read here.
            None => {
                crate::pics::PicsGenerator::new(&*self.db.read().await, &self.config, pics_config)
                    .for_server(self.execution)
                    .generate()
            }
        }
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
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

    /// Get a snapshot of discovery rate-limiting counters.
    pub fn discovery_counters(&self) -> DiscoveryCounters {
        self.discovery_limiter.counters()
    }

    /// Get a snapshot of COV operational and telemetry counters.
    pub fn cov_counters(&self) -> CovCounters {
        self.cov_counters.snapshot()
    }

    /// Sample lifetime mutation authorization decisions, including after `stop()`.
    /// Counts decisions, not completed handlers or response delivery; see
    /// [`MutationDecisionCounters`](crate::mutation::MutationDecisionCounters).
    pub fn mutation_decision_counters(&self) -> crate::mutation::MutationDecisionCounters {
        self.mutation_decisions.snapshot()
    }

    /// Remove COV subscriptions using this current immediate MAC and routed source.
    /// Cleanup of an obsolete router leaves subscriptions renewed onto another route.
    /// Only removed entries release their contribution to the shared quota group.
    /// The table is never waited for in the caller's task, and a call
    /// dropped part way is as [`write_local`](Self::write_local)'s Cancellation section
    /// describes (#2772).
    pub async fn remove_peer_subscriptions(
        &self,
        mac: &[u8],
        network: Option<&NpduAddress>,
    ) -> usize {
        let (owned_mac, owned_network) = (mac.to_vec(), network.cloned());
        let remove = move |mut table: OwnedRwLockWriteGuard<CovSubscriptionTable>| {
            table.remove_peer_subscriptions(&owned_mac, owned_network.as_ref())
        };
        let removed = self
            .request_tasks
            .for_caller(Arc::clone(&self.cov_table).write_owned(), remove)
            .await;
        match removed {
            Some(removed) => removed,
            // Its runtime dropped the task before it removed anything:
            // remove here.
            None => {
                let mut table = self.cov_table.write().await;
                table.remove_peer_subscriptions(mac, network)
            }
        }
    }
}

/// The value `encoded` is for `property` of `oid`, decoded as the
/// WriteProperty handler decodes it.
fn decode_local(
    db: &ObjectDatabase,
    oid: &ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    encoded: &[u8],
) -> Result<PropertyValue, Error> {
    let object = db.get(oid).ok_or(Error::protocol(
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    ))?;
    bacnet_device_core::write::gate_and_decode_write(object, property, array_index, encoded)
}

/// What [`BACnetServer::read_local`] reads with, owned, so a task of the
/// server's can make the read when the database or a table is busy (#2772).
struct LocalRead {
    db: Arc<RwLock<ObjectDatabase>>,
    config: Arc<ServerConfig>,
    execution: crate::device_view::DeviceExecution,
    cov_table: Arc<RwLock<CovSubscriptionTable>>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
    /// The server has stopped: it serves no subscriptions.
    stopped: bool,
}

impl LocalRead {
    async fn read(
        self,
        oid: ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        let db = self.db.read().await;
        let view = crate::device_view::DeviceReadContext::new(&db, self.execution)
            .with_identity(Some(&self.config.device));
        // The local read resolves the Network_Port wildcard to the registered
        // port, but plans a Group's members without it.
        let scope = bacnet_device_core::read::ReadScope {
            work_limit: self
                .config
                .read_property_multiple_budget
                .max_result_elements,
            registered_port: None,
            device: view.device_oid(),
        };
        let lookup_oid = bacnet_device_core::read::resolve_target(
            &db,
            &bacnet_device_core::read::ReadScope {
                registered_port: self.config.registered_network_port,
                ..scope
            },
            &oid,
        );
        let plan = bacnet_device_core::read::plan_property(
            &db,
            &scope,
            Some(&view),
            lookup_oid,
            property,
            array_index,
        )
        .map_err(|failure| match failure {
            bacnet_device_core::read::ReadFailure::Service(error) => error,
            // Work, and any other limit the core reports.
            _ => Error::abort(AbortReason::OUT_OF_RESOURCES),
        })?;
        use super::requests::confirmed_response::{active_cov_snapshot, address_bindings};
        let live = match plan.live_lists(&db) {
            Some(selection) if self.stopped => {
                let bindings = address_bindings(&self.device_bindings, selection).await;
                Some(
                    bacnet_device_core::cov::active::LiveDeviceCov::stopped(selection)
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
        bacnet_device_core::read::read_value(&db, Some(&view), plan)
    }
}

#[cfg(test)]
#[path = "local_encoded_writes_tests.rs"]
mod local_encoded_writes_tests;

#[cfg(test)]
#[path = "caller_waits_tests.rs"]
mod caller_waits_tests;
