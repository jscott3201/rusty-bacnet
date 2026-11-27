//! A binding check's requests (#2825): the task that asks the claimant and
//! the bound address, once, and again a hold-off after a strike.
//!
//! The first check sends the claimant a Who-Is limited to the device's
//! instance, straight to its station or through its router. The second
//! broadcasts it on the bound address's network, or on every network when
//! the claimant is on another ([`broadcast_scope`]), so that the device
//! answering anywhere but the claimant's address is seen. The bound address
//! gets a
//! confirmed ReadProperty of the Device object's Object_Identifier by the
//! wildcard instance, which names the device there; a device that refuses
//! it (one older than the wildcard) is asked for the bound instance's
//! Device object instead, which only that device has. Each read leases an
//! invoke ID as the server's other reads do ([`LeaseMetadata::server_read`]),
//! so only the bound station's answer, through the route's router, ends it,
//! and only silence earns a retry. A send the link fails ends the check
//! undecided: no silence is read from a request that never went out.
//!
//! The task is one of the notification owner's workers, so the server's
//! stop ends it; it holds no lock across a send or a wait, and takes the
//! binding table's guard alone, never under the database's or the COV
//! table's.
//!
//! [`LeaseMetadata::server_read`]: bacnet_endpoint_core::coordinator::LeaseMetadata::server_read

use super::super::super::binding_probes::{WhoIsScope, station_text, targeted_who_is};
use super::super::super::event_recipient_route::{ConfirmedRecipientRoute, RecipientRoute};
use super::super::super::notification_transactions::{
    AnswerListener, Attempt, AttemptsEnd, run_attempts,
};
use super::super::super::remote_writes::{confirmed_request, send};
use super::*;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};

const READ_PROPERTY: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;

/// The server handles a check's task runs on.
pub(in crate::server) struct CheckHandles<T: TransportPort + 'static> {
    pub(in crate::server) network: Arc<NetworkLayer<T>>,
    pub(in crate::server) bindings: Arc<RwLock<DeviceBindingTable>>,
    pub(in crate::server) comm_state: Arc<CommState>,
    pub(in crate::server) transactions: Arc<NotificationTransactions>,
    /// How long each attempt of a read waits for its answer.
    pub(in crate::server) timeout: Duration,
    /// A read's attempts after the first, each earned by silence.
    pub(in crate::server) retries: u8,
    /// The longest APDU this device takes.
    pub(in crate::server) max_apdu: u32,
}

/// Run `start`'s checks on a task of the server's. A server already
/// stopping runs none, and the check ends undecided.
pub(in crate::server) async fn start_check<T: TransportPort + 'static>(
    handles: CheckHandles<T>,
    start: CheckStart,
) {
    let (bindings, transactions) = (
        Arc::clone(&handles.bindings),
        Arc::clone(&handles.transactions),
    );
    if !transactions.spawn(run(handles, start.clone())) {
        let now = runtime_clock::now();
        bindings
            .write()
            .await
            .check_ended(&start, Checked::NOT_ASKED, now);
    }
}

/// What a check's task keeps across its two checks: what the bound station
/// answered.
struct Sequence {
    /// Counts the bound station's answers to the server's requests, from
    /// the first check on.
    listener: Option<AnswerListener>,
    /// How many of those answered the checks' own reads.
    own: u64,
}

async fn run<T: TransportPort + 'static>(handles: CheckHandles<T>, start: CheckStart) {
    let mut sequence = Sequence {
        listener: None,
        own: 0,
    };
    loop {
        let checked = once(&handles, &start, &mut sequence).await;
        let now = runtime_clock::now();
        let step = handles
            .bindings
            .write()
            .await
            .check_ended(&start, checked, now);
        match step {
            Step::Done => return,
            Step::Contested(stations) => {
                let transport = handles.network.transport();
                let stations: Vec<_> = stations
                    .iter()
                    .map(|station| station_text(transport, station))
                    .collect();
                warn!(
                    device = %start.device,
                    claimant = %station_text(transport, &start.claimant),
                    others = %stations.join(", "),
                    "Other stations answered a check's Who-Is for a device beside the station \
                     claiming it: its binding stays where it is"
                );
                return;
            }
            Step::Again => tokio::time::sleep(CHECK_HOLD_OFF).await,
            Step::Moved(from, to) => {
                let transport = handles.network.transport();
                warn!(
                    device = %start.device,
                    from = %station_text(transport, &from),
                    to = %station_text(transport, &to),
                    "A device's binding moved: two checks a minute apart found its old \
                     address failing while the new one answered"
                );
                return;
            }
        }
    }
}

/// One check: the claimant's Who-Is, then the read of the bound address.
async fn once<T: TransportPort + 'static>(
    handles: &CheckHandles<T>,
    start: &CheckStart,
    sequence: &mut Sequence,
) -> Checked {
    if handles.comm_state.initiation_restricted() {
        return Checked::NOT_ASKED;
    }
    let network = &handles.network;
    let is_group = |mac: &[u8]| network.transport().is_group_destination(mac);
    let began = handles
        .bindings
        .write()
        .await
        .check_begins(start, runtime_clock::now(), is_group);
    let local = network.local_network_number().get();
    let Some((route, broadcast)) = began.and_then(|(resolution, second)| {
        // The second check asks everyone the bound address and the claimant
        // can hear, so the device answering elsewhere is seen (decision,
        // maintainer, 2026-11-26).
        let scope = second.then(|| broadcast_scope(&resolution, &start.claimant, local));
        let route = RecipientRoute::from_device_resolution(resolution)
            .localize(local, is_group, is_group)
            .into_confirmed(is_group)
            .ok()?;
        Some((route, scope))
    }) else {
        return Checked::NOT_ASKED;
    };
    let listener = sequence
        .listener
        .get_or_insert_with(|| handles.transactions.listen(route.canonical_peer.clone()));
    if !ask(handles, start, broadcast).await {
        return Checked::NOT_ASKED;
    }
    let asked = tokio::time::Instant::now();
    let identity = identify(handles, &route, start, &mut sequence.own).await;
    if identity.fails(start.device.instance_number()) {
        // The claimant's answer counts for its whole wait.
        tokio::time::sleep_until(asked + start.wait).await;
    }
    let heard = listener
        .answers()
        .is_none_or(|answers| answers > sequence.own);
    Checked { identity, heard }
}

/// Where the second check's Who-Is goes: the network the bound address is
/// on, when the claimant is on it too, as a first contact's Who-Is goes
/// to a lapsed binding's network; every network otherwise.
fn broadcast_scope(
    bound: &DeviceResolution,
    claimant: &DeviceBindingTarget,
    local: Option<u16>,
) -> WhoIsScope {
    let bound = match bound {
        DeviceResolution::ResolvedRouted { network, .. } => WhoIsScope::Remote(*network),
        _ => WhoIsScope::Local,
    };
    let claimant = match claimant {
        DeviceBindingTarget::Local { .. } => WhoIsScope::Local,
        DeviceBindingTarget::Routed { network, .. } => WhoIsScope::Remote(*network),
    };
    let (bound, claimant) = (bound.localize(local), claimant.localize(local));
    if bound == claimant {
        bound
    } else {
        WhoIsScope::Global
    }
}

/// Send a Who-Is limited to the device's instance: broadcast across
/// `broadcast` for the second check, else straight to the claimant, a
/// station on this link or one through its router on another network.
/// Whether it went out. Nothing goes while DeviceCommunicationControl
/// restricts initiation.
async fn ask<T: TransportPort + 'static>(
    handles: &CheckHandles<T>,
    start: &CheckStart,
    broadcast: Option<WhoIsScope>,
) -> bool {
    if handles.comm_state.initiation_restricted() {
        return false;
    }
    let network = &handles.network;
    let apdu = targeted_who_is(start.device);
    let priority = NetworkPriority::Normal;
    if let Some(scope) = broadcast {
        return match scope {
            WhoIsScope::Local => network.broadcast_apdu(&apdu, false, priority).await,
            WhoIsScope::Remote(dnet) => {
                network
                    .broadcast_to_network(&apdu, dnet, false, priority)
                    .await
            }
            WhoIsScope::Global => network.broadcast_global_apdu(&apdu, false, priority).await,
        }
        .is_ok();
    }
    match &start.claimant {
        DeviceBindingTarget::Local { peer_mac } => {
            network.send_apdu(&apdu, peer_mac, false, priority).await
        }
        // A binding through this network's own number is the local
        // station it names (#1358).
        DeviceBindingTarget::Routed {
            network: dnet,
            final_mac,
            ..
        } if Some(*dnet) == network.local_network_number().get() => {
            if network.transport().is_group_destination(final_mac) {
                return false;
            }
            network.send_apdu(&apdu, final_mac, false, priority).await
        }
        DeviceBindingTarget::Routed {
            network: dnet,
            final_mac,
            router_mac,
            ..
        } => {
            network
                .send_apdu_routed(&apdu, *dnet, final_mac, router_mac, false, priority)
                .await
        }
    }
    .is_ok()
}

/// Which device the bound address on `route` says it is: by the wildcard
/// instance, or by the bound instance when that is refused. `own` counts
/// the reads it answered.
async fn identify<T: TransportPort + 'static>(
    handles: &CheckHandles<T>,
    route: &ConfirmedRecipientRoute,
    start: &CheckStart,
    own: &mut u64,
) -> Identity {
    let wildcard = ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE)
        .expect("the wildcard instance is a valid instance");
    match read_identifier(handles, route, start, wildcard, own).await {
        Read::Named(instance) => Identity::Is(instance),
        Read::Silent => Identity::Silent,
        Read::NotAsked => Identity::NotAsked,
        Read::Malformed => Identity::Unclear,
        Read::Refused(_) => match read_identifier(handles, route, start, start.device, own).await {
            Read::Named(instance) => Identity::Is(instance),
            Read::Refused(RemoteError::Error { code, .. }) if code == ErrorCode::UNKNOWN_OBJECT => {
                Identity::NotThere
            }
            Read::NotAsked => Identity::NotAsked,
            // It answered the first read, so its silence here says nothing.
            Read::Silent | Read::Refused(_) | Read::Malformed => Identity::Unclear,
        },
    }
}

/// How one read of a Device object's identifier ended.
enum Read {
    /// The instance its answer holds.
    Named(u32),
    /// An Error, Reject or Abort, with what it said.
    Refused(RemoteError),
    /// An answer that holds no Device identifier.
    Malformed,
    /// Every attempt went out, and none was answered.
    Silent,
    /// Not every attempt went out.
    NotAsked,
}

/// Read `object`'s Object_Identifier at the bound address on `route`, in
/// one of the checks' read slots. `own` counts it when answered.
async fn read_identifier<T: TransportPort + 'static>(
    handles: &CheckHandles<T>,
    route: &ConfirmedRecipientRoute,
    start: &CheckStart,
    object: ObjectIdentifier,
    own: &mut u64,
) -> Read {
    let Ok(_slot) = start.reads.0.acquire().await else {
        return Read::NotAsked;
    };
    let comm_state = &*handles.comm_state;
    if comm_state.initiation_restricted() {
        return Read::NotAsked;
    }
    let mut parameters = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: PropertyIdentifier::OBJECT_IDENTIFIER,
        property_array_index: None,
    }
    .encode(&mut parameters);
    let reserved = handles
        .transactions
        .reserve_read(route.canonical_peer.clone(), READ_PROPERTY);
    let Ok((operation, answer)) = reserved else {
        return Read::NotAsked;
    };
    let invoke_id = operation.invoke_id();
    let apdu = confirmed_request(READ_PROPERTY, invoke_id, handles.max_apdu, parameters);
    route.record_next_hop(&operation);
    let (network, apdu) = (&*handles.network, &apdu);
    let ended = run_attempts(
        operation,
        answer,
        handles.timeout,
        handles.retries,
        |attempt| {
            let withdrawn = comm_state.initiation_restricted()
                || route.freshness.is_some_and(|freshness| {
                    !freshness.permits_attempt_at(tokio::time::Instant::now())
                });
            async move {
                if withdrawn {
                    return Attempt::Withdrawn(());
                }
                match send(network, apdu, route, invoke_id, attempt).await {
                    Ok(()) => Attempt::Sent,
                    Err(()) => Attempt::Withdrawn(()),
                }
            }
        },
    )
    .await;
    let answer = match ended {
        Err(()) | Ok(AttemptsEnd::Closed) => return Read::NotAsked,
        Ok(AttemptsEnd::Exhausted) => return Read::Silent,
        Ok(AttemptsEnd::Answered(answer)) => answer,
    };
    *own += 1;
    match answer {
        CovAckResult::Data(data) => named(&data),
        CovAckResult::Error(refusal) => Read::Refused(refusal),
        CovAckResult::Ack => Read::Malformed,
    }
}

/// The Device instance a ReadProperty-ACK for an Object_Identifier holds.
fn named(data: &[u8]) -> Read {
    let Ok(ack) = ReadPropertyACK::decode(data) else {
        return Read::Malformed;
    };
    if ack.property_identifier != PropertyIdentifier::OBJECT_IDENTIFIER
        || ack.property_array_index.is_some()
    {
        return Read::Malformed;
    }
    match bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0) {
        Ok((PropertyValue::ObjectIdentifier(device), end))
            if end == ack.property_value.len()
                && device.object_type() == ObjectType::DEVICE
                && device.instance_number() != ObjectIdentifier::WILDCARD_INSTANCE =>
        {
            Read::Named(device.instance_number())
        }
        _ => Read::Malformed,
    }
}
