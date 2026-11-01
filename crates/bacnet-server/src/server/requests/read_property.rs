//! The ReadProperty path for an admitted confirmed request (#1713).
//!
//! ReadProperty is the commonest confirmed service, and the general handler's
//! future is sized for every service: several kilobytes, which the allocator
//! serves on its slow path for every request. ReadProperty runs here instead,
//! in a future a fraction of that size. It does what the general handler did
//! for it: a ReadProperty writes nothing, so there are no write effects, COV
//! notifications or command runs to follow its response.
use super::issue::ResponsePlan;
use super::*;
use bacnet_types::error::RemoteError;

/// An admitted confirmed request and everything its handler owns.
pub(in crate::server) struct AdmittedRequest<T: TransportPort + 'static> {
    pub(in crate::server) services: Arc<RequestServices<T>>,
    pub(in crate::server) request_tasks: super::super::request_tasks::RequestTaskSpawner,
    pub(in crate::server) source_mac: MacAddr,
    pub(in crate::server) source_network: Option<NpduAddress>,
    pub(in crate::server) route: bacnet_network::response_route::ResponseRoute,
    pub(in crate::server) req: bacnet_encoding::apdu::ConfirmedRequest,
    pub(in crate::server) reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
    pub(in crate::server) ownership: Option<ConfirmedRequestOwnership>,
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// The task that runs admitted work with its single response owner:
    /// ReadProperty on its own path, every other service in the general
    /// handler, boxed so that handler's size isn't carried by a ReadProperty.
    ///
    /// No async wrapper sits between the spawn and the handler: a wrapper
    /// would hold its own copy of everything it hands the handler.
    pub(in crate::server) fn run_admitted_confirmed_request(
        admitted: AdmittedRequest<T>,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        use futures_util::future::Either;
        if admitted.req.service_choice == ConfirmedServiceChoice::READ_PROPERTY {
            Either::Left(Self::handle_admitted_read_property(admitted))
        } else {
            Either::Right(Box::pin(async move {
                let AdmittedRequest {
                    services,
                    request_tasks,
                    source_mac,
                    source_network,
                    route,
                    req,
                    reply_tx,
                    ownership,
                } = admitted;
                let origin = RequestOrigin {
                    mac: &source_mac,
                    network: source_network,
                    route,
                };
                Self::handle_admitted_confirmed_request(
                    &services,
                    &request_tasks,
                    origin,
                    req,
                    reply_tx,
                    ownership,
                )
                .await;
            }))
        }
    }

    /// Execute an admitted ReadProperty and send its response. The fields of
    /// `admitted` are borrowed where they sit, not moved into locals the
    /// future would hold a second copy of.
    ///
    /// Not an `async fn`: one moves its argument into a local of its own,
    /// so its future held `admitted` twice. The block below captures it once
    /// and uses it in place (#1713).
    #[allow(
        clippy::manual_async_fn,
        reason = "an async fn would hold its moved arguments twice (#1713)"
    )]
    pub(super) fn handle_admitted_read_property(
        mut admitted: AdmittedRequest<T>,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        async move {
            // Fields are read where they sit in `admitted`, borrowed only for
            // as long as each step needs them, and the reply sender and the
            // ownership are taken at the send. Named borrows and early takes
            // would each hold space in this future through the send.
            let (effective_max_apdu, segmented_response_available) = issue::response_limits(
                &admitted.services.config,
                &admitted.route,
                &admitted.req,
                admitted.source_network.as_ref(),
            );
            let audit = super::audit_reporter::WriteAudit::new(
                &admitted.services.config,
                &admitted.services.network,
                &admitted.services.notification_transactions,
                &admitted.services.device_bindings,
                super::audit_reporter::RequestSource {
                    mac: &admitted.source_mac,
                    network: admitted.source_network.as_ref(),
                    invoke_id: admitted.req.invoke_id,
                    local_network: admitted.services.network.local_network_number().get(),
                },
            )
            .await;
            let mut read_audits = Vec::new();
            let config = &admitted.services.config;
            let response = confirmed_response::read_property_response_observed(
                &admitted.services.db,
                Some(admitted.services.live_tables()),
                confirmed_response::ReadProfile {
                    execution: crate::device_view::DeviceExecution::FullServer,
                    registered_port: config.registered_network_port,
                    work_limit: config.read_property_multiple_budget.max_result_elements,
                    identity: Some(&config.device),
                },
                &admitted.req,
                |db, oid, req, result| {
                    let result = match result {
                        Ok(()) => None,
                        Err(
                            Error::Timeout(_)
                            | Error::Remote(RemoteError::Reject { .. })
                            | Error::Remote(RemoteError::Abort { .. }),
                        ) => {
                            return;
                        }
                        Err(error) => Some(bacnet_device_core::reply::error_fields(error)),
                    };
                    read_audits.extend(audit.read_intent(
                        db,
                        oid,
                        req.property_identifier,
                        req.property_array_index,
                        result,
                    ));
                },
            )
            .await;
            Self::issue_confirmed_response(
                ResponsePlan {
                    services: &admitted.services,
                    request_tasks: &admitted.request_tasks,
                    target: ResponseTarget {
                        source_mac: &admitted.source_mac,
                        source_network: admitted.source_network.as_ref(),
                        route: &admitted.route,
                    },
                    ack: ComplexAckParams {
                        invoke_id: admitted.req.invoke_id,
                        service_choice: admitted.req.service_choice,
                        client_max_apdu: effective_max_apdu,
                        client_max_segments: admitted.req.max_segments,
                    },
                    segmented_response_available,
                },
                &response,
                &audit,
                read_audits,
                admitted.reply_tx.take(),
                generic_pending(admitted.ownership.take()),
            )
            .await;
        }
    }
}

/// The pending request `ownership` releases at issuance. Only a
/// LifeSafetyOperation carries replay ownership; dropping one here completes
/// nothing, as the general handler's drop did.
fn generic_pending(
    ownership: Option<ConfirmedRequestOwnership>,
) -> Option<PendingConfirmedRequest> {
    match ownership {
        Some(ConfirmedRequestOwnership::Generic(pending)) => Some(pending),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_transport::bip::BipTransport;

    /// The size of the future `make` returns, without making one.
    fn future_size<A, F: std::future::Future>(_make: fn(A) -> F) -> usize {
        std::mem::size_of::<F>()
    }

    /// The future spawned for every admitted ReadProperty over B/IP is one
    /// allocation per request. Kept within glibc's per-thread cache, whose
    /// largest size class is 1,032 bytes, it is served on the allocator's fast
    /// path; it was 3,248 bytes before #1713. A change that grows it past the
    /// limit should shrink it again (box a rare path, borrow rather than
    /// move, avoid `async fn` for large arguments) rather than raise it.
    #[test]
    fn read_property_handler_future_fits_the_allocator_fast_path() {
        const LIMIT: usize = 1032;
        let read_property =
            future_size(BACnetServer::<BipTransport>::handle_admitted_read_property);
        let spawned = future_size(BACnetServer::<BipTransport>::run_admitted_confirmed_request);
        const ADVICE: &str = "1,032 bytes is glibc's tcache maximum, not a budget; on a \
            toolchain or dependency bump, shrink the future (box a rare path), don't raise the limit";
        eprintln!("ReadProperty handler future: {read_property} bytes; spawned: {spawned} bytes");
        if spawned > LIMIT - 64 {
            eprintln!("warning: the spawned future is within 64 bytes of {LIMIT}. {ADVICE}");
        }
        assert!(
            read_property <= LIMIT,
            "the ReadProperty handler future is {read_property} bytes, over {LIMIT}: {ADVICE}"
        );
        assert!(
            spawned <= LIMIT,
            "the spawned confirmed-request future is {spawned} bytes, over {LIMIT}: {ADVICE}"
        );
    }

    /// The size of the future `make` returns for borrowed arguments, without
    /// making one; a borrow's lifetime doesn't change it.
    fn borrowing_future_size<A: 'static, B: 'static, C: 'static, D: 'static, E: 'static, F>(
        _make: impl FnOnce(&'static A, &'static B, &'static C, &'static mut D, &'static mut E) -> F,
    ) -> usize {
        std::mem::size_of::<F>()
    }

    /// The WriteProperty mutation's future, which holds the write's result
    /// across the COV and Schedule awaits, kept to a ceiling. It runs inside
    /// the general handler's boxed future, not on a path of its own, so no
    /// allocator size class bounds it: the ceiling is its size when the
    /// synchronous write path moved into bacnet-device-core (#2178), 1,024
    /// bytes, plus the 64 bytes of slack the ReadProperty guard warns
    /// within. A change that grows it past the ceiling should shrink it
    /// again (box a rare path, don't hold a large value across an await)
    /// rather than raise it.
    #[test]
    fn write_property_mutation_future_stays_within_its_ceiling() {
        const SIZE_AT_MOVE: usize = 1024;
        const SLACK: usize = 64;
        const CEILING: usize = SIZE_AT_MOVE + SLACK;
        let write_property =
            borrowing_future_size(super::super::mutations::Request::write_property::<BipTransport>);
        const ADVICE: &str = "the ceiling is the size at #2178 plus 64 bytes, not a budget; \
            shrink the future (box a rare path), don't raise the ceiling";
        eprintln!("WriteProperty mutation future: {write_property} bytes; ceiling {CEILING}");
        if write_property > CEILING - SLACK {
            eprintln!("warning: the future is within {SLACK} bytes of {CEILING}. {ADVICE}");
        }
        assert!(
            write_property <= CEILING,
            "the WriteProperty mutation future is {write_property} bytes, over {CEILING}: {ADVICE}"
        );
    }
}
