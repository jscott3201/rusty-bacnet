//! CreateObject and DeleteObject, the services that change which objects
//! the database holds.

use super::*;
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest};

impl Request<'_> {
    /// CreateObject. Once the object is in, the work its arrival queued on
    /// the database (`crate::membership`) is taken under the same guard and
    /// its COV fanout joins the request's.
    pub(in crate::server::requests) async fn create_object<T: TransportPort + 'static>(
        &self,
        services: &RequestServices<T>,
        effects: &mut MutationEffects,
        mut ack_buf: BytesMut,
        audit: &mut crate::server::audit_reporter::WriteAudit<'_, T>,
    ) -> Apdu {
        let (db, cov_table) = (&services.db, &services.cov_table);
        if let Err(error) = self
            .authorize(|| {
                CreateObjectRequest::decode(&self.req.service_request)
                    .map(MutationTarget::CreateObject)
            })
            .await
        {
            return self.error(&error);
        }
        let database = db;
        let (result, membership) = {
            let mut db = db.write().await;
            let mut target = None;
            let result = handlers::handle_create_object_observed(
                &mut db,
                &self.config.object_types,
                &self.req.service_request,
                &mut ack_buf,
                &mut target,
                self.command_origin,
            );
            // Initial application values are decoded inside the handler, possibly
            // after earlier values were applied. Rollback is already complete;
            // malformed values remain invalid requests, not auditable executions.
            let result = match result {
                Ok(()) => Ok(()),
                Err(handlers::CreateObjectRefusal::Malformed(error)) => {
                    return self.error(&error);
                }
                Err(handlers::CreateObjectRefusal::Failed(error)) => Err(error),
            };
            // Decode failures are not execution outcomes. No await separates the
            // completed mutation (including rollback) from audit admission.
            if let Ok(request) = CreateObjectRequest::decode(&self.req.service_request) {
                let kind = match request.object_specifier {
                    bacnet_services::object_mgmt::ObjectSpecifier::Type(kind) => kind,
                    bacnet_services::object_mgmt::ObjectSpecifier::Identifier(oid) => {
                        oid.object_type()
                    }
                };
                audit.before_lifecycle(
                    &db,
                    bacnet_types::enums::AuditOperation::CREATE,
                    target,
                    kind,
                    result.is_ok(),
                );
                audit.lifecycle_completed(&mut db, &result);
            }
            // The objects both change commit their transitions under this
            // guard, counted in the request's settle (#2535).
            let mut transition = committing(|| services.event_delivery(), &mut effects.settle);
            let membership =
                crate::membership::settle(database, &mut db, cov_table, &mut transition).await;
            // A created Schedule runs at once with its initial values, as
            // after a write of it, not at the next minute's pass (#2208).
            let created = target.filter(|_| result.is_ok());
            let schedule = crate::schedule::reevaluate_written(
                database,
                &mut db,
                created.as_slice(),
                cov_table,
                &mut transition,
            )
            .await;
            (result, [membership, schedule])
        };
        for fanout in membership {
            fanout.merge_into(
                &mut effects.coarse_cov_oids,
                &mut effects.life_safety_cov_changes,
                &mut effects.command_runs,
            );
        }
        match result {
            Ok(()) => self.complex_ack(ack_buf),
            Err(e) => self.error(&e),
        }
    }

    /// DeleteObject. The work the removal queued on the database
    /// (`crate::membership`), the relinquishes a deleted Schedule or Staging
    /// object owes its targets included (#2256, #2264, #2265), is taken under
    /// the same guard, and
    /// its COV fanout joins the request's.
    pub(in crate::server::requests) async fn delete_object<T: TransportPort + 'static>(
        &self,
        services: &RequestServices<T>,
        effects: &mut MutationEffects,
        audit: &mut crate::server::audit_reporter::WriteAudit<'_, T>,
    ) -> Apdu {
        let (db, cov_table) = (&services.db, &services.cov_table);
        if let Err(error) = self
            .authorize(|| {
                DeleteObjectRequest::decode(&self.req.service_request)
                    .map(MutationTarget::DeleteObject)
            })
            .await
        {
            return self.error(&error);
        }
        let deleted_oid = DeleteObjectRequest::decode(&self.req.service_request)
            .ok()
            .map(|r| r.object_identifier);
        let database = db;
        let (result, removed, settled) = {
            let mut db = db.write().await;
            let removed_status = deleted_oid.and_then(|oid| {
                db.get(&oid)
                    .and_then(|object| object.audit_reporter())
                    .map(|reporter| reporter.status_internal())
            });
            if let Some(oid) = deleted_oid {
                audit.before_lifecycle(
                    &db,
                    bacnet_types::enums::AuditOperation::DELETE,
                    Some(oid),
                    oid.object_type(),
                    true,
                );
            }
            let (result, removed) = match handlers::delete_object(
                &mut db,
                &self.config.object_types,
                &self.req.service_request,
            ) {
                Ok(removed) => (Ok(()), Some(removed)),
                Err(error) => (Err(error), None),
            };
            if result.is_ok()
                && let Some(status) = removed_status
            {
                status.set_configured(false);
            }
            audit.lifecycle_completed(&mut db, &result);
            // A relinquished target's transition commits under this guard,
            // counted in the request's settle (#2535).
            let mut transition = committing(|| services.event_delivery(), &mut effects.settle);
            let membership =
                crate::membership::settle(database, &mut db, cov_table, &mut transition).await;
            (result, removed, membership)
        };
        settled.merge_into(
            &mut effects.coarse_cov_oids,
            &mut effects.life_safety_cov_changes,
            &mut effects.command_runs,
        );
        // Dropping an object that saves its state waits for its queued saves,
        // so drop it with the guard released and off the async workers.
        if let Some(removed) = removed {
            tokio::task::spawn_blocking(move || drop(removed));
        }
        match result {
            Ok(()) => {
                // Clean up COV subscriptions for the deleted object
                if let Some(oid) = deleted_oid {
                    let mut table = cov_table.write().await;
                    table.remove_for_object(oid);
                }
                self.simple_ack()
            }
            Err(e) => self.error(&e),
        }
    }
}
