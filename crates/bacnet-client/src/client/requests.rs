use super::segmentation_context::{ConfirmedWait, SegmentedRequestLimits};
use super::source_audit::Refused;
use super::*;
use crate::tsm::{CoordinatedRegistrationError, TransactionRegistration};
use bacnet_encoding::apdu::MINIMUM_MESSAGE_SIZE;

#[path = "request_wait.rs"]
mod wait;

/// Which term of Clause 5.2.1.2's minimum bound the transmittable length down.
///
/// Named separately so the error blames the right one. The checked value is a
/// minimum over several terms, and the peer is only sometimes the binding one:
/// BACnet/SC recomputes its own limit from the hub's Connect-Accept, so a
/// transport can fall below the floor while the peer is perfectly conformant.
enum LengthBoundedBy {
    /// A length advertised by a discovered peer, from I-Am.
    DiscoveredPeer(u16),
    /// This client's own configured maximum.
    LocalConfig(u16),
    /// The data link, after any routed NPDU header.
    Transport(u16),
}

impl core::fmt::Display for LengthBoundedBy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DiscoveredPeer(v) => {
                write!(f, "the peer's advertised Max APDU Length Accepted of {v}")
            }
            Self::LocalConfig(v) => write!(f, "this client's configured maximum of {v}"),
            Self::Transport(v) => write!(f, "the transport's limit of {v}"),
        }
    }
}

/// The limits a confirmed request is sent within (`size_request`).
struct RequestSize<'a> {
    /// The largest APDU the request may be.
    remote_max_apdu: u16,
    /// The most segments the peer takes, when its row gives a bound.
    remote_max_segments: Option<u32>,
    /// What the peer's row says of its segmentation, when it has a row.
    peer_segmentation: Option<crate::discovery::PeerSegmentation>,
    /// The network and MAC of the row that sized the request, if one did.
    sized_by: Option<(u16, &'a [u8])>,
}

/// Whether `response` is the device's own answer to a confirmed request: an
/// acknowledgment, of the right kind or not (#2721), or its Error, Reject
/// or Abort. Any of them shows that the request reached the device (Clause
/// 19.4). A router's refusal, a reason 4 ending the request as
/// [`Error::RoutedPathTooLong`] or another reason as
/// [`Error::NetworkRejected`], is no answer from the device.
pub(super) fn answered_by_peer(response: &Result<TsmResponse, Error>) -> bool {
    match response {
        Ok(
            TsmResponse::SimpleAck | TsmResponse::ComplexAck { .. } | TsmResponse::Remote { .. },
        ) => true,
        Ok(TsmResponse::Failed(error)) => matches!(**error, Error::UnexpectedAck { .. }),
        _ => false,
    }
}

/// Reject a maximum transmittable length no conformant device could accept.
///
/// Clause 5.2.1.2 derives this length as the smallest of the local capability,
/// the internetwork limit, and the peer's receive capacity, whose minimum
/// is 50 octets. Below that floor no
/// conformant APDU can be formed at all. Clause 20.1.2.5 gives the same number
/// a name: MinimumMessageSize, the 50-octet capacity represented by the
/// lowest max-APDU-length-accepted code `B'0000'`.
///
/// The check is a floor, not membership of the six values Clause 20.1.2.5
/// encodes. A discovered peer's length comes from I-Am's `Max APDU Length
/// Accepted`, an Unsigned octet count rather than the four-bit code, and
/// Clause 20.1.2.5 allows actual capacity to exceed the encoded value,
/// so 600 and 1500 are legitimate and must not be rejected.
///
/// Failing rather than clamping up to 50 keeps the client from inventing a
/// capability the peer never claimed: a device advertising less than 50 is
/// already non-conformant, and a typed error names that where a silent clamp
/// would send it frames it said it cannot hold.
fn check_transmittable_length(peer_or_local: LengthBoundedBy, transport: u16) -> Result<(), Error> {
    let advertised = match peer_or_local {
        LengthBoundedBy::DiscoveredPeer(v) | LengthBoundedBy::LocalConfig(v) => v,
        LengthBoundedBy::Transport(v) => v,
    };
    let combined = advertised.min(transport);
    if combined >= MINIMUM_MESSAGE_SIZE {
        return Ok(());
    }
    let binding = if transport < advertised {
        LengthBoundedBy::Transport(transport)
    } else {
        peer_or_local
    };
    Err(below_minimum(combined, binding))
}

/// [`check_transmittable_length`]'s refusal, out of line from the request
/// path. The limits are values the client keeps: its own setting, the
/// link's, or the peer's row in its device table (#1833).
#[cold]
#[inline(never)]
fn below_minimum(combined: u16, binding: LengthBoundedBy) -> Error {
    Error::InvalidConfig(Refusal::text(format!(
        "maximum transmittable length {combined} is below the {MINIMUM_MESSAGE_SIZE}-octet \
         MinimumMessageSize every BACnet device accepts (Clause 5.2.1.2); the binding limit is \
         {binding}"
    )))
}

/// A routed request whose path authorization went to another request,
/// refused out of line from the request path (#1833).
#[cold]
#[inline(never)]
fn lost_path_authorization() -> Error {
    Error::limit_reached("routed send lost its active path authorization")
}

/// [`Error::RequestCutOff`], built out of line: the request paths that
/// end with it are hot, and the branch to it is the unlikely one (#1833).
#[cold]
#[inline(never)]
pub(super) fn cut_off(sent: bool) -> Error {
    Error::RequestCutOff { sent }
}

/// The error a request ends with when a send after its first one (a retry,
/// or a segment once an earlier one left) failed with `error` (#1833). A
/// link that stopped under the request refuses it for its state; the
/// request may already have reached the device, so it is cut off as sent,
/// not that refusal, which says nothing was done. Any other failure, a
/// link's I/O error say, is the request's own end as it is.
#[cold]
#[inline(never)]
pub(super) fn retry_refused(error: Error) -> Error {
    match error {
        Error::State(..) => cut_off(true),
        other => other,
    }
}

/// A confirmed request to a group address on this link, refused out of
/// line from the hot request path (#1833).
#[cold]
#[inline(never)]
fn group_destination() -> Error {
    Error::invalid_argument(
        "a confirmed request goes to one device, not to a broadcast or group address (a local \
         broadcast carries only an UNCONFIRMED_REQUEST APDU, Clause 6.3)",
    )
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send a confirmed request to the station `to` and wait for its answer:
    /// the service data of its acknowledgement (empty for a SimpleAck). The
    /// typed calls are this with their request encoded and their answer
    /// checked. A request larger than the device takes is sent segmented
    /// when it accepts segments.
    ///
    /// `to` is a MAC on this link, a [`BACnetAddress`] or a [`Station`]
    /// ([`ToStation`] lists the forms). A station on another network goes
    /// through the router the caller names or, for an address, the one the
    /// discovery table's row at it names (Clause 6.2.2), with its network
    /// and MAC as the NPDU's DNET and DADR. Once the client has learned its
    /// own network's number, a station on it is sent to at its MAC on this
    /// link, with no DNET and not through any router (#1358); only an answer
    /// from that MAC with no SNET completes the request, not one relayed
    /// back with that network and MAC as its SNET and SADR, which any
    /// station could send (#2831). The row at the station's address,
    /// when the table has one, bounds the request by the device's Max APDU
    /// Length Accepted and segmentation, and an answer from it starts the
    /// row's age again; with no row the client's configured limits apply.
    ///
    /// # Errors
    ///
    /// Before any request state is taken: [`Error::InvalidArgument`] for a station
    /// a confirmed request can't go to. That is a MAC that reaches a group
    /// of nodes on this link ([`TransportPort::is_group_destination`]: the
    /// link's broadcast MAC, or on B/IP the limited broadcast, the
    /// configured broadcast IP or a multicast address at any port), which
    /// would make it a local broadcast (Clause 6.3, #1479); an empty MAC in
    /// an address; and on another network a network of 0 or 65535 or a MAC
    /// longer than [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN) octets, which
    /// no router carries to one station, a MAC that is a broadcast or a
    /// group on that network ([`RemoteBroadcast`], #2709), or a
    /// named router that is a broadcast or group address on this link
    /// (#2722). [`Error::UnknownDevice`] for a remote address with no named
    /// router and no row in the discovery table, since the row is the route.
    /// Then the errors of the request itself; a request the client's stop,
    /// or its link stopping or losing its connection after the request went
    /// out, cuts off fails with [`Error::RequestCutOff`]: `sent: false`
    /// while it waited for its turn, `sent: true` once it was open.
    ///
    /// [`BACnetAddress`]: bacnet_types::constructed::BACnetAddress
    pub async fn confirmed_request(
        &self,
        to: &(impl ToStation + ?Sized),
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        to.confirmed(self, service_choice, service_data, None).await
    }

    /// Register a confirmed request with the TSM, its lock held for the
    /// call alone. The segmented request matches on the result where it is
    /// made, without naming it, so its future doesn't keep the result
    /// through its later awaits (#2617).
    #[inline(always)]
    pub(super) fn register_transaction(
        &self,
        tsm_mac: MacAddr,
        canonical: CanonicalPeer,
        service_choice: ConfirmedServiceChoice,
        segmented: bool,
    ) -> Result<(u8, TransactionRegistration), CoordinatedRegistrationError> {
        self.tsm
            .lock()
            .register_coordinated_transaction_with_progress(
                tsm_mac,
                canonical,
                service_choice,
                segmented,
            )
    }

    /// The one request path: `attributes` are the data attributes each
    /// frame carries (#2723), `None` for none.
    ///
    /// The checks and endings it calls are its own code moved out (#2558),
    /// and their attributes were measured on the ReadProperty, WriteProperty
    /// and ReadPropertyMultiple round trips, on both runtimes, in local
    /// `scripts/perf/perf_counts.py` runs against e5061ee1b with the
    /// hash-map keys fixed (#2591). `send_target` and `size_request` are
    /// `#[inline(always)]`: without it, the pace guard's drop costs 15
    /// instructions more, and each request 5.5 to 8.8 more.
    /// `check_segmented_send` and `abandon_unsent`, which a request that
    /// goes unsegmented and is sent never reaches, are `#[cold]
    /// #[inline(never)]`: without all four attributes,
    /// `cap_max_apdu_to_transport` is no longer inlined here, and each
    /// request costs 26.6 to 30.6 more.
    pub(super) async fn confirmed_request_inner(
        &self,
        target: ConfirmedTarget<'_>,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
        attributes: Attributes<'_>,
    ) -> Result<Bytes, Error> {
        // The target is checked and localized first (`send_target`); the
        // peer's limits still come from the device-table row of the target
        // as named.
        let named = target;
        let (target, routed_forwarded_npci_len) = self.send_target(named)?;
        // Pace before the path lease, so the wait holds up no other request
        // through the same router. The guard lives until this request
        // returns, by any path or by being dropped, and then starts the
        // interval for the next request to this destination. A request still
        // waiting when the client's tasks end (its stop, or its node's stop
        // or drop) ends then, as the open requests do, and claims no turn
        // (#2720). A client with no interval names no destination and
        // waits for nothing, and neither here does one whose node audits
        // its requests: the node takes the turn once it admits the request
        // (#2652).
        let _pace = if self.pacer.paces_in_request_path() {
            let key = pacing::PaceKey::of(target);
            match self.pacer.wait(key, &self.routing_query.ended).await {
                Some(pace) => pace,
                // Still waiting for its turn: nothing has gone out.
                None => return Err(cut_off(false)),
            }
        } else {
            pacing::PaceGuard::unpaced()
        };
        // The lease serializes one active request per (immediate router, DNET)
        // and remains live through every return path, including cancellation.
        // A request by local broadcast names no router, so it takes none.
        let mut path_lease = match target.routed_path() {
            None => None,
            Some((router_mac, dest_network)) => Some(
                self.routed_path_limits
                    .acquire(router_mac, dest_network)
                    .await?,
            ),
        };
        let transaction_peer = target.transaction_peer();
        let tsm_mac = transaction_peer.tsm_mac;
        // Opened before the TSM holds the request, so that an answer can be
        // told by the path it came through from the first (#2831).
        let routed_path_max_apdu = path_lease
            .as_ref()
            .zip(routed_forwarded_npci_len)
            .map(|(lease, npci_len)| lease.open(&tsm_mac, npci_len));
        let unsegmented_apdu_size = 4 + service_data.len();
        let RequestSize {
            remote_max_apdu,
            remote_max_segments,
            peer_segmentation,
            sized_by,
        } = self.size_request(named, target, routed_path_max_apdu)?;
        if unsegmented_apdu_size > remote_max_apdu as usize {
            self.check_segmented_send(peer_segmentation)?;
            let mut limits = SegmentedRequestLimits {
                named: &named,
                remote_max_apdu,
                remote_max_segments,
                routed_forwarded_npci_len,
                routed_path_lease: &mut path_lease,
                sized_by,
                attributes,
            };
            return self
                .segmented_confirmed_request(target, service_choice, service_data, &mut limits)
                .await;
        }

        let advertised_max_apdu = self.advertised_max_apdu_length_for_target(target)?;
        // The TSM keys below copy `tsm_mac` with `from_slice`, which copies
        // it at once; `clone` copies a `MacAddr` a byte at a time (#1978).
        let registered = self.register_transaction(
            MacAddr::from_slice(&tsm_mac),
            transaction_peer.canonical,
            service_choice,
            false,
        );
        // A refusal is a closed seat, the node's DeviceCommunicationControl,
        // or a request the node audits (#2617).
        let (invoke_id, registration) = match registered {
            Ok(registered) => registered,
            Err(refused) => match self.on_refused(refused, target, service_choice, false)? {
                Refused::Registered(registered) => registered,
                Refused::Audited(auditor) => {
                    // The runner takes the routed path again.
                    drop(path_lease);
                    let request = self.audited_request(
                        auditor,
                        named,
                        service_choice,
                        service_data,
                        attributes,
                    );
                    return request.await;
                }
            },
        };

        let owner = registration.owner.clone();
        // The transaction keeps the request's data attributes, so its sends
        // carry them from there (#2723).
        owner.set_data_attributes(attributes);
        let mut guard = TransactionGuard::new(
            Arc::clone(&self.tsm),
            self.cleanup_tx.clone(),
            MacAddr::from_slice(&tsm_mac),
            invoke_id,
            owner.clone(),
            None,
        );

        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: self.config.receives_segments(),
            max_segments: self.config.max_segments,
            max_apdu_length: advertised_max_apdu,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(6 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        if let Some(forwarded_npci_len) = routed_forwarded_npci_len {
            self.routed_path_limits.install_active(
                target,
                &tsm_mac,
                invoke_id,
                owner.clone(),
                forwarded_npci_len,
                self.network.network_control_ingress_sequence(),
            );
        }
        if !self.routed_path_limits.authorize_attempt(
            target,
            &tsm_mac,
            invoke_id,
            &owner,
            buf.len(),
        ) {
            guard.mark_completed();
            self.abandon_unsent(&tsm_mac, invoke_id, &owner);
            // The path's one active request is no longer this one.
            return Err(lost_path_authorization());
        }

        if let Err(e) = self.send_confirmed_target_apdu(target, &buf, &owner).await {
            guard.mark_completed();
            self.abandon_unsent(&tsm_mac, invoke_id, &owner);
            return Err(e);
        }

        let mut response = self
            .wait_for_confirmed_response(
                ConfirmedWait {
                    target,
                    tsm_mac: &tsm_mac,
                    invoke_id,
                    owner: &owner,
                    retry_apdu: Some(&buf),
                },
                registration.response,
                registration.progress,
            )
            .await;
        // Checked here too, so a request no row sized makes no call.
        if sized_by.is_some() {
            self.note_answer(sized_by, &response);
        }
        if let Some(lease) = path_lease.as_ref() {
            self.settle_routed(lease, (&tsm_mac, invoke_id, &owner), &mut response);
        }
        guard.mark_completed();
        response.and_then(Self::confirmed_response_result)
    }

    /// Check a request's target and localize it, before anything is taken
    /// for the request. A destination that is not one device (#1278), or a
    /// DADR or local source MAC that no router could carry (#1267), fails
    /// here, before the path gate is reserved or awaited and before TSM
    /// registration. A routed target on this client's own network is then
    /// sent as the local target it is (#1358). Returns the target to send
    /// to and, for one still routed, the length of the NPCI its router
    /// forwards.
    #[inline(always)]
    fn send_target<'a>(
        &self,
        named: ConfirmedTarget<'a>,
    ) -> Result<(ConfirmedTarget<'a>, Option<u16>), Error> {
        if let ConfirmedTarget::Routed {
            dest_network,
            dest_mac,
            ..
        } = named
        {
            check_routed_unicast(dest_network, dest_mac.len())?;
        }
        let target = named.localized(self.network.local_network_number().get());
        match target {
            ConfirmedTarget::Local { mac } => {
                // An empty MAC names no one station: refused as an address
                // on network 0 with no MAC is, whichever form named it
                // (#2588).
                Self::station_of(0, mac)?;
                // With no DNET, a group destination is a local broadcast,
                // which carries no confirmed request (Clause 6.3, #1479).
                if self.network.transport().is_group_destination(mac) {
                    return Err(group_destination());
                }
            }
            // Still routed, so on another network: its MAC there must not
            // spell that network's broadcast (#2709), as the station's
            // one-station mark has it (#2738), and the router it goes
            // through must be one station on this link (#2722). The local
            // broadcast form names no router; its DNET and DADR name the
            // one station, so they are all it is checked by.
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
                one_station,
            } => {
                send_checks::check_remote_station(dest_network, dest_mac, one_station)?;
                send_checks::check_router(self.network.transport(), dest_network, router_mac)?;
            }
        }
        let routed_forwarded_npci_len = match target {
            ConfirmedTarget::Local { .. } => None,
            ConfirmedTarget::Routed { dest_mac, .. } => {
                Some(forwarded_npci_len(dest_mac.len(), self.local_mac.len())?)
            }
        };
        Ok((target, routed_forwarded_npci_len))
    }

    /// How large a request to `target` may be: the smallest of the peer's
    /// Max APDU Length Accepted (from the discovery-table row of the target
    /// as `named`, or this client's own maximum with no row), the link's
    /// limit and a routed path's, `routed_path_max_apdu`. Fails when that
    /// is below what any device accepts.
    #[inline(always)]
    fn size_request<'a>(
        &self,
        named: ConfirmedTarget<'a>,
        target: ConfirmedTarget<'a>,
        routed_path_max_apdu: Option<u16>,
    ) -> Result<RequestSize<'a>, Error> {
        let target_transport_max_apdu = self.target_transport_max_apdu_length(target);

        let (peer_max_apdu, remote_max_segments, advertised, peer_segmentation, sized_by) = {
            let dt = self.device_table.lock();
            // A routed peer's row is keyed by the SNET/SADR of the NPDU that
            // carried its I-Am, never by the router's MAC (Clause 5.2.1.2
            // term (c) binds the peer's Max APDU Length Accepted regardless
            // of how the peer is reached).
            // A routed target on this client's own network is the local
            // device it is sent to, whose row sizes it when it has no
            // routed row (#1358).
            let mut sized_by = named.station();
            let mut found = dt.peer(sized_by.0, sized_by.1);
            if found.is_none()
                && let (ConfirmedTarget::Routed { .. }, ConfirmedTarget::Local { mac }) =
                    (named, target)
            {
                sized_by = (0, mac);
                found = dt.peer(0, mac);
            }
            let (device, peer_segmentation) = found.unzip();
            let max_apdu = device
                .map(|d| u16::try_from(d.max_apdu_length).unwrap_or(u16::MAX))
                .unwrap_or(self.config.max_apdu_length);
            let max_seg = device.and_then(|d| d.max_segments_accepted);
            let advertised = if device.is_some() {
                LengthBoundedBy::DiscoveredPeer(max_apdu)
            } else {
                LengthBoundedBy::LocalConfig(max_apdu)
            };
            let sized_by = device.is_some().then_some(sized_by);
            (max_apdu, max_seg, advertised, peer_segmentation, sized_by)
        };
        check_transmittable_length(advertised, target_transport_max_apdu)?;
        if routed_path_max_apdu.is_some_and(|path_max_apdu| {
            path_max_apdu < MINIMUM_MESSAGE_SIZE
                && path_max_apdu <= peer_max_apdu.min(target_transport_max_apdu)
        }) {
            let ConfirmedTarget::Routed { dest_network, .. } = target else {
                unreachable!("a routed path limit exists only for a routed target")
            };
            return Err(Error::RoutedPathTooLong { dnet: dest_network });
        }
        let remote_max_apdu = peer_max_apdu
            .min(target_transport_max_apdu)
            .min(routed_path_max_apdu.unwrap_or(u16::MAX));
        Ok(RequestSize {
            remote_max_apdu,
            remote_max_segments,
            peer_segmentation,
            sized_by,
        })
    }

    /// A request too long for one APDU goes segmented, unless this client
    /// sends no segments or the peer, by its own account, takes none.
    #[cold]
    #[inline(never)]
    fn check_segmented_send(
        &self,
        peer_segmentation: Option<crate::discovery::PeerSegmentation>,
    ) -> Result<(), Error> {
        // A client that sends no segments (a node whose Device says so,
        // #2617) refuses first, whatever the peer takes.
        self.config.check_segmented_request()?;
        // Clause 12.11: a NO_SEGMENTATION or SEGMENTED_TRANSMIT peer
        // accepts exactly one segment — only unsegmented requests — and
        // Clause 18's SEGMENTATION_NOT_SUPPORTED abort is the certain
        // outcome of sending anyway. Refuse locally when the capability
        // is authoritative (I-Am or explicit configuration); a legacy
        // placeholder row stays unknown and keeps today's behavior.
        if let Some(crate::discovery::PeerSegmentation::Authoritative(capability)) =
            peer_segmentation
            && matches!(
                capability,
                bacnet_types::enums::Segmentation::NO_SEGMENTATION
                    | bacnet_types::enums::Segmentation::SEGMENTED_TRANSMIT
            )
        {
            return Err(
                bacnet_types::error::SegmentationError::PeerCannotReceive { capability }.into(),
            );
        }
        Ok(())
    }

    /// A request that could not be sent ends here: its transaction leaves
    /// the TSM, and the dispatch task drops what it holds for it.
    #[cold]
    #[inline(never)]
    fn abandon_unsent(&self, tsm_mac: &MacAddr, invoke_id: u8, owner: &TransactionOwner) {
        self.tsm
            .lock()
            .cancel_transaction_for_owner(tsm_mac, invoke_id, owner);
        self.enqueue_transaction_cleanup(tsm_mac, invoke_id, owner, false, None);
    }

    /// An answer from the device ([`answered_by_peer`]) starts the age of
    /// the discovery-table row that sized the request, `sized_by`, again, so
    /// a device that keeps answering stays in the table without repeating
    /// its I-Am (#2560). With no such row the table isn't locked again.
    pub(super) fn note_answer(
        &self,
        sized_by: Option<(u16, &[u8])>,
        response: &Result<TsmResponse, Error>,
    ) {
        if let Some((network, mac)) = sized_by
            && answered_by_peer(response)
        {
            self.device_table.lock().answered(network, mac);
        }
    }

    pub(super) fn enqueue_transaction_cleanup(
        &self,
        mac: &MacAddr,
        invoke_id: u8,
        owner: &TransactionOwner,
        cancel_tsm: bool,
        seg_ack_sender: Option<mpsc::Sender<SegmentAckPdu>>,
    ) {
        let _ = self.cleanup_tx.send(TransactionCleanup {
            mac: mac.clone(),
            invoke_id,
            owner: owner.clone(),
            cancel_tsm,
            seg_ack_sender,
        });
    }

    pub(super) fn confirmed_response_result(response: TsmResponse) -> Result<Bytes, Error> {
        response.into_result()
    }

    /// Send one frame of a confirmed request (its first, a retry or a
    /// segment) with the data attributes it was made with
    /// ([`BACnetClient::with_data_attributes`]), which its transaction's
    /// `owner` keeps, none by default: to the station's MAC, to its router,
    /// or by one broadcast on this link for the form that names no router.
    pub(super) async fn send_confirmed_target_apdu(
        &self,
        target: ConfirmedTarget<'_>,
        apdu: &[u8],
        owner: &TransactionOwner,
    ) -> Result<(), Error> {
        let priority = NetworkPriority::Normal;
        let attributes = owner.data_attributes();
        match target {
            ConfirmedTarget::Local { mac } => {
                self.network
                    .send_apdu_with_data_attributes(apdu, mac, true, priority, attributes)
                    .await
            }
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
                ..
            } if router_mac == station::VIA_LOCAL_BROADCAST => {
                self.network
                    .send_apdu_routed_via_local_broadcast_with_data_attributes(
                        apdu,
                        dest_network,
                        dest_mac,
                        true,
                        priority,
                        attributes,
                    )
                    .await
            }
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
                ..
            } => {
                let to = bacnet_network::layer::RoutedTarget {
                    network: dest_network,
                    mac: dest_mac,
                    router_mac,
                };
                self.network
                    .send_apdu_routed_with_data_attributes(apdu, to, true, priority, attributes)
                    .await
            }
        }
    }

    pub(super) fn target_transport_max_apdu_length(&self, target: ConfirmedTarget<'_>) -> u16 {
        self.network
            .transport()
            .egress_apdu_limit()
            .saturating_sub(target.additional_npdu_header_len())
    }

    pub(super) fn advertised_max_apdu_length_for_target(
        &self,
        target: ConfirmedTarget<'_>,
    ) -> Result<u16, Error> {
        cap_max_apdu_to_transport(
            self.config.max_apdu_length,
            self.target_transport_max_apdu_length(target),
        )
    }

    /// Before the first Who-Is, of any scope, ask this network's number
    /// (#2572): the I-Ams about to fill the discovery table may include a
    /// device a router relays back from this network. Only one frame goes
    /// ahead of the Who-Is, and a question that fails to send holds nothing
    /// back. The seat is checked again after it: the node may have stopped,
    /// or restricted initiation, while the question went out (#2617).
    pub(super) async fn ask_before(
        &self,
        service_choice: UnconfirmedServiceChoice,
    ) -> Result<(), Error> {
        if service_choice == UnconfirmedServiceChoice::WHO_IS {
            self.device_table.asking.first_need(&self.network).await;
            self.seat_open()?;
        }
        Ok(())
    }

    /// Send an unconfirmed request to the station `to`, once, with no answer
    /// waited for: success means it was sent.
    ///
    /// `to` takes the forms [`Self::confirmed_request`] takes and is routed
    /// the same way. A MAC on this link is sent to as given; an address on
    /// this client's own network, once it has learned the number, is the
    /// station at that MAC on this link, which must then be one station, not
    /// a broadcast or group address.
    ///
    /// # Errors
    ///
    /// As [`Self::confirmed_request`] before sending, and the transport's
    /// error.
    pub async fn unconfirmed_request(
        &self,
        to: &(impl ToStation + ?Sized),
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        to.unconfirmed(self, service_choice, service_data).await
    }

    /// An unconfirmed request to `destination_mac` on this link: one
    /// station, so an empty MAC, which names none, is refused as an address
    /// on network 0 with no MAC is (#2588).
    pub(super) async fn unconfirmed_local(
        &self,
        destination_mac: &[u8],
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        Self::station_of(0, destination_mac)?;
        self.seat_open()?;
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        self.ask_before(service_choice).await?;
        self.network
            .send_apdu(&buf, destination_mac, false, NetworkPriority::Normal)
            .await
    }

    /// Broadcast an unconfirmed request on the local network.
    pub async fn broadcast_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        self.seat_open()?;
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        self.ask_before(service_choice).await?;
        self.network
            .broadcast_apdu(&buf, false, NetworkPriority::Normal)
            .await
    }

    /// Broadcast an unconfirmed request globally (DNET=0xFFFF).
    pub async fn broadcast_global_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        self.seat_open()?;
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        self.ask_before(service_choice).await?;
        self.network
            .broadcast_global_apdu(&buf, false, NetworkPriority::Normal)
            .await
    }

    /// Broadcast an unconfirmed request to a specific remote network.
    ///
    /// Once the client has learned its own network's number, a
    /// `dest_network` naming it is this network, and the request goes as a
    /// local broadcast with no DNET (#1358).
    pub async fn broadcast_network_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
        dest_network: u16,
    ) -> Result<(), Error> {
        self.seat_open()?;
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu)?;

        // Network 0 and 65535 are refused below, before anything goes: a
        // refused Who-Is asks nothing.
        if matches!(dest_network, 1..=0xFFFE) {
            self.ask_before(service_choice).await?;
        }
        if self.network.local_network_number().get() == Some(dest_network) {
            return self
                .network
                .broadcast_apdu(&buf, false, NetworkPriority::Normal)
                .await;
        }
        self.network
            .broadcast_to_network(&buf, dest_network, false, NetworkPriority::Normal)
            .await
    }
}

#[cfg(test)]
#[path = "requests_tests.rs"]
mod transmittable_length_tests;

#[cfg(test)]
#[path = "request_send_tests.rs"]
mod send_tests;
