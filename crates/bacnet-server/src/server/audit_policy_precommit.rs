//! Mandatory built-in policy changes share the existing immediate admission owner.
use super::*;
use bacnet_types::primitives::BACnetTimeStamp;

fn denied() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
    }
}

impl<T: TransportPort + 'static> WriteAudit<'_, T> {
    pub(super) fn commit_mandatory_policy(
        &mut self,
        db: &mut ObjectDatabase,
        write: WriteTarget<'_>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        self.pending.as_ref()?.selection.as_ref()?.change?;
        // Read the clock and obtain the concrete capability before admission:
        // neither custom object nor clock callbacks execute inside commit locks.
        let clock = db
            .clock_frame()
            .filter(|frame| frame.is_valid_actual_datetime());
        let sequence = db.event_sequence_internal();
        let candidate = db
            .get_mut(&write.oid)?
            .audit_policy_authority_internal()?
            .prepare(write.property, write.array_index, value, write.priority)
            .ok()??;
        // Invalid/equal/NULL attempts retain the ordinary writer and observer.
        // An actual mandatory change exclusively owns its result from here on.
        let mut pending = self.pending.take().expect("selected mandatory change");
        let owner = self.transactions.audit_owner_lease();
        let commit = |number| {
            self.transactions
                .commit_audit(|| {
                    if !owner.as_ref().is_some_and(|owner| owner.is_active())
                        || self.comm_state.load(Ordering::Acquire) != 0
                    {
                        return Err(denied());
                    }
                    let route = pending.route.take().ok_or_else(denied)?;
                    let permit = self.transactions.try_admit_audit().map_err(|_| denied())?;
                    let reserved = if pending.confirmed {
                        Some(
                            self.transactions
                                .reserve(
                                    route.canonical_peer.clone(),
                                    ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                                )
                                .map_err(|_| denied())?,
                        )
                    } else {
                        None
                    };
                    pending.notification.target_timestamp = Some(match clock {
                        Some(frame) => BACnetTimeStamp::DateTime {
                            date: frame.local_date,
                            time: frame.local_time,
                        },
                        None => BACnetTimeStamp::SequenceNumber(number),
                    });
                    let invoke = reserved
                        .as_ref()
                        .map_or(0, |(operation, _)| operation.invoke_id());
                    let bytes = encode_notification(
                        &pending.notification,
                        pending.confirmed,
                        self.config.max_apdu_length,
                        invoke,
                    )
                    .ok_or_else(denied)?;
                    let network = Arc::clone(self.network);
                    let comm_state = Arc::clone(self.comm_state);
                    let deadline = tokio::time::Instant::now() + DELIVERY_TIMEOUT;
                    let completion = DeliveryCompletion {
                        status: pending.status,
                        epoch: pending.completion,
                        finished: false,
                    };
                    // All fallible preparation is complete. This sealed assignment
                    // cannot call arbitrary object code, allocate or return an error.
                    candidate.commit();
                    Ok(async move {
                        let _permit = permit;
                        completion.finish(
                            deliver(&network, &comm_state, &route, &bytes, reserved, deadline)
                                .await,
                        );
                    })
                })
                .map_err(|_| denied())
        };
        Some(if clock.is_some() {
            commit(0)
        } else {
            sequence.transaction(commit)
        })
    }
}
