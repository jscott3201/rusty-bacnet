use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Get the server's local MAC address.
    pub fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    /// Get a reference to the shared object database.
    ///
    /// Objects read through this handle return standalone object data. In
    /// particular, the Device's `Active_COV_Subscriptions` is always its empty
    /// default list: the live list is owned by the server's COV subscription
    /// table. Use [`read_local`](Self::read_local) for the live server view.
    pub fn database(&self) -> &Arc<RwLock<ObjectDatabase>> {
        &self.db
    }

    /// Read one local property through the server's ReadProperty evaluator.
    ///
    /// This is the live local read boundary: it applies the same Device
    /// wildcard resolution, `UNKNOWN_OBJECT` and `PROPERTY_IS_NOT_AN_ARRAY`
    /// checks, and value source as network ReadProperty. The selected Device's
    /// `Active_COV_Subscriptions` is projected from the COV subscription table
    /// at one sampled instant (ordinary and single-property subscriptions only),
    /// returned as its encoded `BACnetLIST of BACnetCOVSubscription`
    /// (`PropertyValue::ApplicationData`). After [`stop`](Self::stop) the
    /// server services no subscriptions, so that property reads as an empty list.
    pub async fn read_local(
        &self,
        oid: &ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        let db = self.db.read().await;
        let lookup_oid = handlers::resolve_device_wildcard(&db, oid);
        let live = match handlers::active_cov_device(&db, lookup_oid, property) {
            Some(device) if self.dispatch_task.is_none() => {
                Some(crate::cov::active::ActiveCovSubscriptions::stopped(device))
            }
            Some(device) => Some(
                super::requests::confirmed_response::active_cov_snapshot(
                    &db,
                    &self.cov_table,
                    device,
                )
                .await,
            ),
            None => None,
        };
        handlers::read_property_value(&db, live.as_ref(), lookup_oid, property, array_index)
    }

    /// Create a cloneable handle for unsolicited I-Am announcements.
    pub fn i_am_broadcaster(&self) -> IAmBroadcaster<T> {
        IAmBroadcaster {
            config: self.config.clone(),
            network: Arc::clone(&self.network),
            db: Arc::clone(&self.db),
        }
    }

    /// Get the communication state per DeviceCommunicationControl.
    ///
    /// Returns 0 (Enable), 1 (Disable), or 2 (DisableInitiation).
    pub fn comm_state(&self) -> u8 {
        self.comm_state.load(Ordering::Acquire)
    }

    /// Generate a PICS document from the current object database and server configuration.
    ///
    /// The caller must supply a [`PicsConfig`] for fields not available from the server
    /// (vendor name, model, firmware revision, etc.).
    pub async fn generate_pics(&self, pics_config: &crate::pics::PicsConfig) -> crate::pics::Pics {
        let db = self.db.read().await;
        crate::pics::PicsGenerator::new(&db, &self.config, pics_config).generate()
    }
}
