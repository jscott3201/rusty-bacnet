//! Immediate best-effort forwarding after durable receipt, never an outbox.

use super::device_bindings::BindingFreshness;
use super::event_recipient_route::{ConfirmedRecipientRoute, RecipientRoute};
use super::notification_transactions::{run_notification_worker, NotificationWorkerResult};
use super::*;
use bacnet_objects::audit::AuditLogForwarding;
use bacnet_objects::database::LocalDevice;

const DEADLINE: Duration = Duration::from_secs(3);

/// The configured route to the parent Audit Log's device. The parent must
/// name a device other than this one ([`ObjectDatabase::local_device`]), and
/// this device must have a concrete identity to tell the two apart. A
/// binding routed through `local_network`, this network's own number when
/// known, is taken as the local one it is (#1358), so the copy goes to the
/// parent's MAC with no DNET. A binding at a group address of the link
/// (`is_group`) gives no route, so no copy goes there (#1493).
fn resolve(
    profile: &AuditLogForwarding,
    local: LocalDevice,
    bindings: &DeviceBindingTable,
    local_mac: &[u8],
    local_network: Option<u16>,
    is_group: impl Fn(&[u8]) -> bool,
) -> Option<ConfirmedRecipientRoute> {
    let parent = profile.parent();
    let device = parent.device_identifier?;
    if device.object_type() != ObjectType::DEVICE
        || local.identifier().is_none()
        || local.is_local(Some(device))
        || parent.object_identifier.object_type() != ObjectType::AUDIT_LOG
    {
        return None;
    }
    let route = RecipientRoute::from_device_resolution(bindings.resolve_at(
        &device,
        runtime_clock::now(),
        &is_group,
    ))
    .localize(local_network, &is_group, &is_group)
    .into_confirmed(&is_group)
    .ok()?;
    if route.freshness != Some(BindingFreshness::Configured)
        || route.local_target.as_deref() == Some(local_mac)
    {
        return None;
    }
    Some(route)
}

pub(super) fn initialize<T: TransportPort>(
    db: &ObjectDatabase,
    config: &ServerConfig,
    bindings: &DeviceBindingTable,
    transport: &T,
) {
    if let Some(profile) = config
        .audit_notification_sink
        .filter(|sink| sink.object_type() == ObjectType::AUDIT_LOG)
        .and_then(|sink| db.get(&sink))
        .and_then(|object| object.audit_log_forwarding_internal())
    {
        // Before the link starts, while no local network number is known.
        profile.status().set_configured(
            resolve(
                &profile,
                db.local_device(),
                bindings,
                transport.local_mac(),
                None,
                |mac| transport.is_group_destination(mac),
            )
            .is_some(),
        );
    }
}

pub(super) struct ForwardBatch {
    profile: Arc<AuditLogForwarding>,
    local: LocalDevice,
    payload: Bytes,
}

impl ForwardBatch {
    pub(super) fn after_commit(
        db: &ObjectDatabase,
        sink: ObjectIdentifier,
        changed: bool,
        payload: Bytes,
    ) -> Option<Self> {
        if !changed {
            return None;
        }
        Some(Self {
            profile: db.get(&sink)?.audit_log_forwarding_internal()?,
            local: db.local_device(),
            payload,
        })
    }

    // Called only after the database guard has been released. Admission never
    // waits, and workers belong to the existing joined shutdown owner. A
    // forward is a ConfirmedAuditNotification, which Clause 16.1 leaves
    // running under DISABLE_INITIATION, so DCC state never holds it back.
    pub(super) fn start<T: TransportPort + 'static>(
        self,
        network: &Arc<NetworkLayer<T>>,
        transactions: &Arc<NotificationTransactions>,
        bindings: &Arc<RwLock<DeviceBindingTable>>,
        max_apdu: u32,
    ) {
        let completion = Completion::new(Arc::clone(&self.profile));
        let deadline = tokio::time::Instant::now() + DEADLINE;
        // No queue, waiting permit, retry, or AuditingFailure producer.
        let Ok(permit) = transactions.try_admit_audit() else {
            return;
        };
        // Resolve synchronously before spawn: contention is a failed best-effort
        // attempt, not permission to accumulate tasks waiting for bindings.
        let Ok(bindings) = bindings.try_read() else {
            return;
        };
        let route = resolve(
            &self.profile,
            self.local,
            &bindings,
            network.local_mac(),
            network.local_network_number().get(),
            |mac| network.transport().is_group_destination(mac),
        );
        drop(bindings);
        self.profile.status().set_configured(route.is_some());
        let Some(route) = route else {
            return;
        };
        // Four-octet unsegmented confirmed header; preserve the original service
        // bytes (including every optional notification field), not merged records.
        if self.payload.len().saturating_add(4) > max_apdu as usize {
            return;
        }
        let Ok((operation, receiver)) = transactions.reserve(
            route.canonical_peer.clone(),
            ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        ) else {
            return;
        };
        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: apdu::max_apdu_header_at_or_below(max_apdu)
                .expect("validated local APDU capacity"),
            invoke_id: operation.invoke_id(),
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            service_request: self.payload,
        });
        let mut bytes = BytesMut::new();
        if encode_apdu(&mut bytes, &pdu).is_err() {
            return;
        }
        let network = Arc::clone(network);
        transactions.spawn(async move {
            let _permit = permit;
            let delivered = tokio::time::timeout_at(
                deadline,
                run_notification_worker(operation, receiver, DEADLINE, 0, |_| async {
                    // timeout_at polls its inner future first. Do not initiate
                    // I/O if scheduling consumed the entire delivery budget.
                    if tokio::time::Instant::now() >= deadline {
                        return Err(Error::Encoding("audit forwarding deadline expired".into()));
                    }
                    match (&route.local_target, &route.remote) {
                        (Some(mac), None) => {
                            network
                                .send_apdu(&bytes, mac, true, NetworkPriority::NORMAL)
                                .await
                        }
                        (None, Some((net, mac, Some(router)))) => {
                            network
                                .send_apdu_routed(
                                    &bytes,
                                    *net,
                                    mac,
                                    router,
                                    true,
                                    NetworkPriority::NORMAL,
                                )
                                .await
                        }
                        _ => Err(Error::Encoding("audit forwarding route unavailable".into())),
                    }
                }),
            )
            .await
            .is_ok_and(|result| result == NotificationWorkerResult::Ack);
            completion.finish(delivered);
        });
    }
}

struct Completion {
    profile: Arc<AuditLogForwarding>,
    epoch: bacnet_objects::audit::AuditDeliveryToken,
    finished: bool,
}

impl Completion {
    fn new(profile: Arc<AuditLogForwarding>) -> Self {
        Self {
            epoch: profile.status().begin_delivery(),
            profile,
            finished: false,
        }
    }
    fn finish(mut self, success: bool) {
        self.profile.status().complete_delivery(self.epoch, success);
        self.finished = true;
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        if !self.finished {
            self.profile.status().complete_delivery(self.epoch, false);
        }
    }
}
