use super::*;
use bacnet_services::who_am_i::WhoAmIRequest;
use bacnet_services::who_has::{WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::{DeviceInstanceRange, WhoIsRequest};

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// The station the discovery table holds for `device_instance`, for any
    /// typed call: [`Station::Local`] at the row's MAC, or
    /// [`Station::Routed`] through the router the row's I-Am came through
    /// (#2588). A call by instance is this and the call:
    ///
    /// ```no_run
    /// # use bacnet_client::client::BACnetClient;
    /// # use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    /// # use bacnet_types::primitives::ObjectIdentifier;
    /// # async fn example(client: &BACnetClient) -> Result<(), bacnet_types::error::Error> {
    /// let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    /// let device = client.resolve_device(1234).await?;
    /// let ack = client
    ///     .read_property(&device, object, PropertyIdentifier::PRESENT_VALUE, None)
    ///     .await?;
    /// # Ok(()) }
    /// ```
    ///
    /// The lookup comes first, so its error comes before any the request's
    /// own checks would give.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more addresses claim the
    /// instance (the device's address names it then), and an error naming
    /// the instance when the table holds none.
    ///
    /// # Snapshot
    ///
    /// The station is the row as it was under one [`DeviceTable`] lock,
    /// released before anything is sent. A request to it takes the row at
    /// its address again for the device's Max APDU and segmentation, so a
    /// table update in between can size the request by a newer row than
    /// the one that placed it; holding the lock across the request would
    /// serialize every request.
    pub async fn resolve_device(&self, device_instance: u32) -> Result<Station, Error> {
        let table = self.device_table.lock();
        let device = table.find(device_instance)?.ok_or_else(|| {
            Error::Encoding(format!("device {device_instance} not in device table"))
        })?;
        Ok(Station::from(device))
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Send a Who-Is to `scope` to discover devices: [`Broadcast::Global`]
    /// reaches devices behind routers too, [`Broadcast::Local`] this
    /// network alone, [`Broadcast::Network`] one remote network.
    ///
    /// `range` limits the devices that answer to those whose instance lies
    /// in it; `None` asks every device. A [`DeviceInstanceRange`] holds both
    /// limits, low no greater than high, so the request can't carry one
    /// alone (Clause 16.10, #1483).
    ///
    /// The client's first Who-Is, of any scope, goes right after one
    /// What-Is-Network-Number while the client doesn't know its network's
    /// number (#2572): the answer lets a device a router relays back from
    /// this network be one device. A question that fails to send doesn't
    /// hold the Who-Is back.
    pub async fn who_is(
        &self,
        scope: Broadcast,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        self.tell_all(scope, WhoIsRequest { range }).await
    }

    /// Send a directed (unicast) WhoIs to the station `to`, with the same
    /// `range` as [`Self::who_is`].
    pub async fn who_is_directed(
        &self,
        to: &(impl ToStation + ?Sized),
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        self.tell(to, WhoIsRequest { range }).await
    }

    /// Send a Who-Has to `scope` to find an object by identifier or name,
    /// asking the devices in `range`, or every device for `None` (Clause
    /// 16.9, #1483). [`Broadcast::Global`] reaches devices behind routers
    /// too.
    pub async fn who_has(
        &self,
        scope: Broadcast,
        object: WhoHasObject,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        self.tell_all(scope, WhoHasRequest { range, object }).await
    }

    /// Send a Who-Has to the station `to` alone, as [`Self::who_has`]
    /// does to a scope.
    pub async fn who_has_directed(
        &self,
        to: &(impl ToStation + ?Sized),
        object: WhoHasObject,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        self.tell(to, WhoHasRequest { range, object }).await
    }

    /// Send a Who-Am-I to `scope` (Clause 16.11): this device asks to be
    /// given an identity, naming itself by the vendor, model name and
    /// serial number in `request`, which should be its Device object's.
    /// A model name or serial number that can't be encoded fails before
    /// anything is sent.
    ///
    /// Each call sends one; pacing repeats is the caller's. The standard
    /// lets a device send a Who-Am-I at any time but not repeat it more
    /// often than once in five minutes, unless it answers a Who-Is or an
    /// operator asks for it (Clause 16.11.2).
    pub async fn who_am_i(&self, scope: Broadcast, request: &WhoAmIRequest) -> Result<(), Error> {
        self.tell_all(scope, request).await
    }

    /// Send a Who-Am-I to the station `to` alone, as [`Self::who_am_i`]
    /// does to a scope: to answer a Who-Is from it, say.
    pub async fn who_am_i_directed(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &WhoAmIRequest,
    ) -> Result<(), Error> {
        self.tell(to, request).await
    }

    /// Get a snapshot of all discovered devices, one per address, ordered by
    /// network number and then MAC. An instance two addresses claim is
    /// listed twice.
    pub async fn discovered_devices(&self) -> Vec<DiscoveredDevice> {
        self.device_table.lock().all()
    }

    /// Look up the discovered device that claims `instance`, or `Ok(None)`
    /// when no address does.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`], naming every claimant's address, when two
    /// or more addresses claim the instance. [`Self::get_device_at`] finds
    /// each of them.
    pub async fn get_device(&self, instance: u32) -> Result<Option<DiscoveredDevice>, Error> {
        Ok(self.device_table.lock().find(instance)?.cloned())
    }

    /// Look up the discovered device at `mac` on `network`: network 0 is the
    /// local network; on a remote network `mac` is the device's MAC there,
    /// not its router's.
    pub async fn get_device_at(&self, network: u16, mac: &[u8]) -> Option<DiscoveredDevice> {
        self.device_table.lock().get(network, mac).cloned()
    }

    /// Clear the discovered devices table.
    pub async fn clear_devices(&self) {
        self.device_table.lock().clear();
    }

    /// Manually register a local device in the device table.
    ///
    /// Useful for adding known devices without requiring WhoIs/IAm exchange.
    /// The metadata defaults below (max_apdu_length 1476, segmentation NO_SEGMENTATION,
    /// vendor_id 0) are local/manual registration defaults, **not** values
    /// learned from an I-Am; requests to this device are bounded by them
    /// until a real I-Am refreshes the row. The row is unambiguously local:
    /// routed lookups by SNET/SADR will not match it. Like a discovered
    /// row, it is one claim on `instance`: another address claiming it too
    /// makes the instance ambiguous (see [`DeviceTable`]).
    ///
    /// The table holds one device per address: registering `instance` at
    /// a MAC that holds another instance replaces that row, so
    /// [`Self::get_device`] no longer finds the instance it held there.
    pub async fn add_device(&self, instance: u32, mac: &[u8]) -> Result<(), Error> {
        let oid = bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::DEVICE,
            instance,
        )?;
        let device = DiscoveredDevice {
            object_identifier: oid,
            mac_address: MacAddr::from_slice(mac),
            max_apdu_length: 1476,
            segmentation_supported: bacnet_types::enums::Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
            vendor_id: 0,
            last_seen: crate::discovery::now(),
            last_announced: None,
            source_network: None,
            source_address: None,
        };
        // Placeholder capability defaults (NO_SEGMENTATION, 1476) are not I-Am evidence;
        // they must never drive a segmentation-capability decision (#371).
        self.device_table.lock().upsert_placeholder(device);
        Ok(())
    }

    /// Manually register a routed peer whose address and capabilities are
    /// already known, without fabricating an I-Am exchange.
    ///
    /// `router_mac` is only the immediate transport next hop (Clause 6.2.2);
    /// the peer's identity is the (`remote_network`, `remote_mac`) SNET/SADR
    /// pair, which is what a request to a routed [`Station`] and the
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
    /// unchanged (an empty SADR could never satisfy a routed lookup). A
    /// `remote_network` outside 1..=65534 or a `remote_mac` longer than
    /// [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN) is refused the same way, since
    /// a routed confirmed request ([`BACnetClient::confirmed_request`])
    /// refuses that destination.
    ///
    /// A `remote_network` that is this client's own network, once the client
    /// has learned its number, names a local device: it is stored as the
    /// local row at `remote_mac`, as requests to it go out locally (#1358).
    /// One registered before the number was learned became that local row
    /// when it was (#2572). Like the other manual registrations, this sends
    /// no device or collision notices. A routed device registered while the
    /// number is unknown has the client ask for it first, once (#2572).
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
        check_routed_unicast(config.remote_network, config.remote_mac.len())?;
        let oid = bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::DEVICE,
            config.instance,
        )?;
        // A station on this client's own network is the local device at that
        // MAC (#1358). The number is read under the lock its learning folds
        // the table under (#2572): a routed row registered before then has
        // become the local row, and none is registered after.
        let this_network = {
            let mut table = self.device_table.lock();
            let this_network = self.network.local_network_number().get();
            let local = this_network == Some(config.remote_network);
            let remote_mac = MacAddr::from_slice(&config.remote_mac);
            let (mac_address, source_network, source_address) = if local {
                (remote_mac, None, None)
            } else {
                let router = MacAddr::from_slice(&config.router_mac);
                (router, Some(config.remote_network), Some(remote_mac))
            };
            let device = DiscoveredDevice {
                object_identifier: oid,
                mac_address,
                max_apdu_length: config.max_apdu_length,
                segmentation_supported: config.segmentation_supported,
                max_segments_accepted: config.max_segments_accepted,
                vendor_id: 0,
                last_seen: crate::discovery::now(),
                last_announced: None,
                source_network,
                source_address,
            };
            if local {
                table.upsert_by_number(device);
            } else {
                table.upsert(device);
            }
            this_network
        };
        // A routed row stored while the number is unknown may be a local
        // device's twin: ask, once (#2572).
        if this_network.is_none() {
            self.device_table.asking.first_need(&self.network).await;
        }
        Ok(())
    }
}
