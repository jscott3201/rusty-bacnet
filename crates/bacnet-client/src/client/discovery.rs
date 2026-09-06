use super::*;
use bacnet_types::enums::Segmentation;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Resolve a device instance to its MAC address and optional routing info.
    ///
    /// # Snapshot/coherence contract
    ///
    /// This returns an address/routing snapshot taken under one
    /// [`DeviceTable`] lock; the lock is released before any network I/O.
    /// The request's later capability lookup (Max APDU/segmentation) takes a
    /// second lock, so a table update between the two reads can make the
    /// capability decision observe a newer entry than the address snapshot.
    /// This is an intentional non-atomic snapshot boundary, not a promise
    /// that both reads use the same row forever; holding the lock across the
    /// request would serialize all in-flight requests.
    pub(super) async fn resolve_device(
        &self,
        device_instance: u32,
    ) -> Result<(Vec<u8>, Option<(u16, Vec<u8>)>), Error> {
        let dt = self.device_table.lock().await;
        let device = dt.get(device_instance).ok_or_else(|| {
            Error::Encoding(format!("device {device_instance} not in device table"))
        })?;
        let routing = match (&device.source_network, &device.source_address) {
            (Some(snet), Some(sadr)) => Some((*snet, sadr.to_vec())),
            _ => None,
        };
        Ok((device.mac_address.to_vec(), routing))
    }

    // -----------------------------------------------------------------------
    // Multi-device batch operations
    // -----------------------------------------------------------------------

    /// Read a property from multiple discovered devices concurrently.
    ///
    /// All requests are dispatched concurrently (up to `max_concurrent`,
    /// default 32) and results are returned in completion order. Each device
    /// is resolved from the device table and auto-routed if behind a router.
    /// Send a WhoIs broadcast to discover devices.
    pub async fn who_is(
        &self,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> Result<(), Error> {
        use bacnet_services::who_is::WhoIsRequest;

        let request = WhoIsRequest {
            low_limit,
            high_limit,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.broadcast_global_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &buf)
            .await
    }

    /// Send a directed (unicast) WhoIs to a specific device.
    pub async fn who_is_directed(
        &self,
        destination_mac: &[u8],
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> Result<(), Error> {
        use bacnet_services::who_is::WhoIsRequest;

        let request = WhoIsRequest {
            low_limit,
            high_limit,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.unconfirmed_request(destination_mac, UnconfirmedServiceChoice::WHO_IS, &buf)
            .await
    }

    /// Send a WhoIs broadcast to a specific remote network.
    pub async fn who_is_network(
        &self,
        dest_network: u16,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> Result<(), Error> {
        use bacnet_services::who_is::WhoIsRequest;

        let request = WhoIsRequest {
            low_limit,
            high_limit,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.broadcast_network_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &buf, dest_network)
            .await
    }

    /// Send a WhoHas broadcast to find an object by identifier or name.
    pub async fn who_has(
        &self,
        object: bacnet_services::who_has::WhoHasObject,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> Result<(), Error> {
        use bacnet_services::who_has::WhoHasRequest;

        let request = WhoHasRequest {
            low_limit,
            high_limit,
            object,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        self.broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_HAS, &buf)
            .await
    }

    /// Subscribe to COV notifications for an object on a remote device.
    /// Get a snapshot of all discovered devices.
    pub async fn discovered_devices(&self) -> Vec<DiscoveredDevice> {
        self.device_table.lock().await.all()
    }

    /// Look up a discovered device by instance number.
    pub async fn get_device(&self, instance: u32) -> Option<DiscoveredDevice> {
        self.device_table.lock().await.get(instance).cloned()
    }

    /// Clear the discovered devices table.
    pub async fn clear_devices(&self) {
        self.device_table.lock().await.clear();
    }

    /// Manually register a local device in the device table.
    ///
    /// Useful for adding known devices without requiring WhoIs/IAm exchange.
    /// The metadata defaults below (max_apdu_length 1476, segmentation NONE,
    /// vendor_id 0) are local/manual registration defaults, **not** values
    /// learned from an I-Am; requests to this device are bounded by them
    /// until a real I-Am refreshes the row. The row is unambiguously local:
    /// routed lookups by SNET/SADR will not match it.
    pub async fn add_device(&self, instance: u32, mac: &[u8]) -> Result<(), Error> {
        let oid = bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::DEVICE,
            instance,
        )?;
        let device = DiscoveredDevice {
            object_identifier: oid,
            mac_address: MacAddr::from_slice(mac),
            max_apdu_length: 1476,
            segmentation_supported: bacnet_types::enums::Segmentation::NONE,
            max_segments_accepted: None,
            vendor_id: 0,
            last_seen: std::time::Instant::now(),
            source_network: None,
            source_address: None,
        };
        // Placeholder capability defaults (NONE, 1476) are not I-Am evidence;
        // they must never drive a segmentation-capability decision (#371).
        self.device_table.lock().await.upsert_placeholder(device);
        Ok(())
    }

    /// Manually register a routed peer whose address and capabilities are
    /// already known, without fabricating an I-Am exchange.
    ///
    /// `router_mac` is only the immediate transport next hop (Clause 6.2.2);
    /// the peer's identity is the (`remote_network`, `remote_mac`) SNET/SADR
    /// pair, which is what [`BACnetClient::confirmed_request_routed`] and the
    /// device-table routed lookup match against. The supplied Max APDU
    /// Length Accepted, segmentation capability, and optional Max Segments
    /// Accepted are treated as advertised peer limits for request sizing.
    ///
    /// Vendor ID is recorded as 0 when unknown; this does not affect
    /// routing or request-size decisions.
    ///
    /// Malformed routing metadata is rejected deliberately: an empty
    /// `router_mac` or `remote_mac` cannot identify a next hop or a peer, so
    /// such a registration returns [`Error::Encoding`] and the table is left
    /// unchanged (an empty SADR could never satisfy a routed lookup).
    pub async fn add_routed_device(&self, config: RoutedDeviceConfig) -> Result<(), Error> {
        if config.router_mac.is_empty() {
            return Err(Error::Encoding(
                "router_mac must not be empty: it is the immediate transport next hop".into(),
            ));
        }
        if config.remote_mac.is_empty() {
            return Err(Error::Encoding(
                "remote_mac must not be empty: it is half of the routed peer's identity".into(),
            ));
        }
        let oid = bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::DEVICE,
            config.instance,
        )?;
        let device = DiscoveredDevice {
            object_identifier: oid,
            mac_address: MacAddr::from_slice(&config.router_mac),
            max_apdu_length: config.max_apdu_length,
            segmentation_supported: config.segmentation_supported,
            max_segments_accepted: config.max_segments_accepted,
            vendor_id: 0,
            last_seen: std::time::Instant::now(),
            source_network: Some(config.remote_network),
            source_address: Some(MacAddr::from_slice(&config.remote_mac)),
        };
        self.device_table.lock().await.upsert(device);
        Ok(())
    }
}
