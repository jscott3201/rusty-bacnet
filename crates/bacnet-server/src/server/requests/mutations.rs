use super::*;
use crate::life_safety_cov::LifeSafetyCovChange;
use crate::mutation::{MutationAuthorizationContext, MutationTarget};
use bacnet_objects::staging::StagingWritePlan;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;
use bacnet_services::file::AtomicWriteFileRequest;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest};
use bacnet_services::write_property::WritePropertyRequest;

pub(super) enum InitialCovNotification {
    Single(CovSubscription),
    Multiple(Vec<CovSubscription>),
}

/// Borrowed dispatch inputs; constructed only after the DCC precheck.
pub(super) struct Request<'a> {
    pub config: &'a ServerConfig,
    pub source_mac: &'a [u8],
    pub source_network: Option<&'a NpduAddress>,
    pub req: &'a ConfirmedRequestPdu,
}

impl Request<'_> {
    fn authorize(
        &self,
        decode: impl FnOnce() -> Result<MutationTarget, Error>,
    ) -> Result<(), Error> {
        let Some(authorizer) = &self.config.mutation_authorizer else {
            return Ok(());
        };
        let context = MutationAuthorizationContext {
            source_mac: MacAddr::from_slice(self.source_mac),
            source_network: self.source_network.cloned(),
            invoke_id: self.req.invoke_id,
            service_choice: self.req.service_choice,
            target: decode()?,
        };
        if audit_notification::fail_closed_authorize(|| authorizer(&context)) {
            Ok(())
        } else {
            Err(audit_notification::request_denied())
        }
    }

    fn error<T: TransportPort + 'static>(&self, error: &Error) -> Apdu {
        BACnetServer::<T>::error_apdu_from_error(self.req.invoke_id, self.req.service_choice, error)
    }

    fn simple_ack(&self) -> Apdu {
        Apdu::SimpleAck(SimpleAck {
            invoke_id: self.req.invoke_id,
            service_choice: self.req.service_choice,
        })
    }

    fn complex_ack(&self, ack_buf: BytesMut) -> Apdu {
        Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: self.req.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: self.req.service_choice,
            service_ack: ack_buf.freeze(),
        })
    }

    pub(super) async fn write_property<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        written_oids: &mut Vec<ObjectIdentifier>,
        coarse_cov_oids: &mut Vec<ObjectIdentifier>,
        life_safety_cov_changes: &mut Vec<LifeSafetyCovChange>,
        staging_plans: &mut Vec<StagingWritePlan>,
    ) -> Apdu {
        if let Err(error) = self.authorize(|| {
            WritePropertyRequest::decode(&self.req.service_request)
                .map(MutationTarget::WriteProperty)
        }) {
            return self.error::<T>(&error);
        }
        let (result, exact_changes, plans) = {
            let mut db = db.write().await;
            let snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::capture_write_property(
                &db,
                &self.req.service_request,
            );
            let result = handlers::handle_write_property(&mut db, &self.req.service_request);
            let changes = result
                .as_ref()
                .map(|oid| snapshots.changes(&db, std::slice::from_ref(oid)))
                .unwrap_or_default();
            let plans = result.as_ref().map_or_else(
                |_| Vec::new(),
                |oid| BACnetServer::<T>::take_staging_plans(&mut db, std::slice::from_ref(oid)),
            );
            (result, changes, plans)
        };
        staging_plans.extend(plans);
        match result {
            Ok(oid) => {
                written_oids.push(oid);
                if crate::life_safety_cov::is_life_safety_object(oid) {
                    *life_safety_cov_changes = exact_changes;
                } else {
                    coarse_cov_oids.push(oid);
                }
                self.simple_ack()
            }
            Err(e) => self.error::<T>(&e),
        }
    }

    pub(super) async fn write_property_multiple<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        written_oids: &mut Vec<ObjectIdentifier>,
        coarse_cov_oids: &mut Vec<ObjectIdentifier>,
        life_safety_cov_changes: &mut Vec<LifeSafetyCovChange>,
        staging_plans: &mut Vec<StagingWritePlan>,
    ) -> Apdu {
        let (outcome, exact_changes, plans) = {
            let mut db = db.write().await;
            let mut snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::default();
            let authorize = |attempt: &bacnet_services::wpm::WritePropertyAttempt| {
                self.authorize(|| Ok(MutationTarget::WritePropertyMultiple(attempt.clone())))
            };
            let outcome = handlers::handle_write_property_multiple_authorized(
                &mut db,
                &self.req.service_request,
                &mut snapshots,
                self.config
                    .mutation_authorizer
                    .as_ref()
                    .map(|_| &authorize as _),
            );
            let committed_oids = match &outcome {
                handlers::WritePropertyMultipleOutcome::Success { committed_oids }
                | handlers::WritePropertyMultipleOutcome::Error { committed_oids, .. } => {
                    committed_oids.as_slice()
                }
                handlers::WritePropertyMultipleOutcome::Reject { .. } => &[],
            };
            let changes = snapshots.changes(&db, committed_oids);
            let plans = BACnetServer::<T>::take_staging_plans(&mut db, committed_oids);
            (outcome, changes, plans)
        };
        staging_plans.extend(plans);
        let response = match outcome {
            handlers::WritePropertyMultipleOutcome::Success { committed_oids } => {
                *written_oids = committed_oids;
                self.simple_ack()
            }
            handlers::WritePropertyMultipleOutcome::Error {
                error,
                first_failed_write_attempt,
                committed_oids,
            } => {
                *written_oids = committed_oids;
                let (error_class, error_code) = confirmed_response::error_fields(&error);
                Apdu::Error(
                    bacnet_services::wpm::WritePropertyMultipleError {
                        error_class,
                        error_code,
                        first_failed_write_attempt,
                    }
                    .to_error_pdu(self.req.invoke_id),
                )
            }
            handlers::WritePropertyMultipleOutcome::Reject { reason } => Apdu::Reject(RejectPdu {
                invoke_id: self.req.invoke_id,
                reject_reason: reason,
            }),
        };
        coarse_cov_oids.extend(
            written_oids
                .iter()
                .copied()
                .filter(|oid| !crate::life_safety_cov::is_life_safety_object(*oid)),
        );
        *life_safety_cov_changes = exact_changes;
        response
    }

    pub(super) async fn subscribe_cov<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        initial_cov_notifications: &mut Vec<InitialCovNotification>,
    ) -> Apdu {
        if let Err(error) = self.authorize(|| {
            SubscribeCOVRequest::decode(&self.req.service_request).map(MutationTarget::SubscribeCov)
        }) {
            return self.error::<T>(&error);
        }
        let db = db.read().await;
        let mut table = cov_table.write().await;
        match handlers::handle_subscribe_cov_with_initial_endpoint(
            &mut table,
            &db,
            self.source_mac,
            self.source_network,
            &self.req.service_request,
        ) {
            Ok(subscriptions) => {
                initial_cov_notifications.extend(
                    subscriptions
                        .into_iter()
                        .map(InitialCovNotification::Single),
                );
                self.simple_ack()
            }
            Err(e) => self.error::<T>(&e),
        }
    }

    pub(super) async fn subscribe_cov_property<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        initial_cov_notifications: &mut Vec<InitialCovNotification>,
    ) -> Apdu {
        if let Err(error) = self.authorize(|| {
            SubscribeCOVPropertyRequest::decode(&self.req.service_request)
                .map(MutationTarget::SubscribeCovProperty)
        }) {
            return self.error::<T>(&error);
        }
        let db = db.read().await;
        let mut table = cov_table.write().await;
        match handlers::handle_subscribe_cov_property_with_initial_endpoint(
            &mut table,
            &db,
            self.source_mac,
            self.source_network,
            &self.req.service_request,
        ) {
            Ok(subscriptions) => {
                initial_cov_notifications.extend(
                    subscriptions
                        .into_iter()
                        .map(InitialCovNotification::Single),
                );
                self.simple_ack()
            }
            Err(e) => self.error::<T>(&e),
        }
    }

    pub(super) async fn create_object<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        mut ack_buf: BytesMut,
    ) -> Apdu {
        if let Err(error) = self.authorize(|| {
            CreateObjectRequest::decode(&self.req.service_request).map(MutationTarget::CreateObject)
        }) {
            return self.error::<T>(&error);
        }
        let result = {
            let mut db = db.write().await;
            handlers::handle_create_object(&mut db, &self.req.service_request, &mut ack_buf)
        };
        match result {
            Ok(()) => self.complex_ack(ack_buf),
            Err(e) => self.error::<T>(&e),
        }
    }

    pub(super) async fn delete_object<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
    ) -> Apdu {
        if let Err(error) = self.authorize(|| {
            DeleteObjectRequest::decode(&self.req.service_request).map(MutationTarget::DeleteObject)
        }) {
            return self.error::<T>(&error);
        }
        let deleted_oid = DeleteObjectRequest::decode(&self.req.service_request)
            .ok()
            .map(|r| r.object_identifier);
        let result = {
            let mut db = db.write().await;
            handlers::handle_delete_object(&mut db, &self.req.service_request)
        };
        match result {
            Ok(()) => {
                // Clean up COV subscriptions for the deleted object
                if let Some(oid) = deleted_oid {
                    let mut table = cov_table.write().await;
                    table.remove_for_object(oid);
                }
                self.simple_ack()
            }
            Err(e) => self.error::<T>(&e),
        }
    }

    pub(super) async fn atomic_write_file<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
    ) -> Apdu {
        if let Err(error) = self.authorize(|| {
            AtomicWriteFileRequest::decode(&self.req.service_request)
                .map(MutationTarget::AtomicWriteFile)
        }) {
            return self.error::<T>(&error);
        }
        let mut db = db.write().await;
        BACnetServer::<T>::atomic_write_file_response(
            &mut db,
            self.req.invoke_id,
            &self.req.service_request,
            self.config.atomic_write_file_budget,
        )
    }

    pub(super) async fn add_list_element<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
    ) -> Apdu {
        if let Err(error) = self.authorize(|| {
            ListElementRequest::decode(&self.req.service_request)
                .map(MutationTarget::AddListElement)
        }) {
            return self.error::<T>(&error);
        }
        let mut db = db.write().await;
        match handlers::handle_add_list_element(&mut db, &self.req.service_request) {
            Ok(()) => self.simple_ack(),
            Err(e) => self.error::<T>(&e),
        }
    }

    pub(super) async fn remove_list_element<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
    ) -> Apdu {
        if let Err(error) = self.authorize(|| {
            ListElementRequest::decode(&self.req.service_request)
                .map(MutationTarget::RemoveListElement)
        }) {
            return self.error::<T>(&error);
        }
        let mut db = db.write().await;
        match handlers::handle_remove_list_element(&mut db, &self.req.service_request) {
            Ok(()) => self.simple_ack(),
            Err(e) => self.error::<T>(&e),
        }
    }

    pub(super) async fn subscribe_cov_property_multiple<T: TransportPort + 'static>(
        &self,
        db: &Arc<RwLock<ObjectDatabase>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        initial_cov_notifications: &mut Vec<InitialCovNotification>,
    ) -> Apdu {
        let decoded = SubscribeCOVPropertyMultipleRequest::decode(&self.req.service_request);
        match decoded {
            Err(e) => self.error::<T>(&e),
            Ok(request) => {
                if let Err(error) = self.authorize(|| {
                    Ok(MutationTarget::SubscribeCovPropertyMultiple(
                        request.clone(),
                    ))
                }) {
                    return self.error::<T>(&error);
                }
                let db = db.read().await;
                let mut table = cov_table.write().await;
                match handlers::handle_subscribe_cov_property_multiple_request_endpoint(
                    &mut table,
                    &db,
                    self.source_mac,
                    self.source_network,
                    request,
                ) {
                    Ok(subscriptions) => {
                        if !subscriptions.is_empty() {
                            initial_cov_notifications
                                .push(InitialCovNotification::Multiple(subscriptions));
                        }
                        self.simple_ack()
                    }
                    Err(e) => self.error::<T>(&e),
                }
            }
        }
    }
}
