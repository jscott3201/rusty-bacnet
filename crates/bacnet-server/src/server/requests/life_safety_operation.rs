//! An admitted LifeSafetyOperation: decoded, validated, its target looked
//! up, the server policy asked, and then carried out under the database's
//! write guard. Moved out of the general handler's body (#2558); its checks
//! run in the same order, and its replay entry is still stored by the
//! handler once the response exists.
use super::other_services::ServiceCall;
use super::*;

/// Execute `call`'s LifeSafetyOperation and build its response. The COV
/// changes it makes join `effects`; `provenance` goes into the context the
/// policy is asked with.
pub(super) async fn response<T: TransportPort + 'static>(
    call: &ServiceCall<'_, T>,
    provenance: bacnet_transport::port::TransportProvenance,
    effects: &mut MutationEffects,
) -> Apdu {
    let ServiceCall {
        services,
        req,
        source_mac,
        source_network,
        invoke_id,
        service_choice,
        ..
    } = *call;
    let RequestServices {
        db,
        cov_table,
        config,
        ..
    } = &**services;
    let request =
        bacnet_services::life_safety::LifeSafetyOperationRequest::decode(&req.service_request)
            .map_err(Error::into_request_reject);
    match request {
        Err(e) => bacnet_device_core::reply::error_apdu(invoke_id, service_choice, &e),
        Ok(request) => {
            let validation = handlers::validate_life_safety_operation(request.request);
            let execution = if let Err(e) = validation {
                Err(e)
            } else {
                let target_exists = match request.object_identifier {
                    Some(oid) => db.read().await.get(&oid).is_some(),
                    None => true,
                };

                if !target_exists {
                    Err(handlers::life_safety_error(
                        ErrorClass::OBJECT,
                        ErrorCode::UNKNOWN_OBJECT,
                    ))
                } else {
                    let context = LifeSafetyOperationAuthorizationContext {
                        provenance,
                        source_mac: MacAddr::from_slice(source_mac),
                        source_network: source_network.clone(),
                        invoke_id,
                        request: request.clone(),
                    };
                    let authorized = match &config.policy {
                        Some(policy) => {
                            ask_policy(policy, context, |policy, context| {
                                policy.authorize_life_safety_operation(context)
                            })
                            .await
                        }
                        None => false,
                    };
                    if !authorized {
                        Err(handlers::life_safety_error(
                            ErrorClass::SERVICES,
                            ErrorCode::SERVICE_REQUEST_DENIED,
                        ))
                    } else {
                        let mut db = db.write().await;
                        let result = handlers::handle_life_safety_operation(&mut db, &request);
                        // Timestamped references capture the
                        // exact changes under this guard (#856).
                        if let Ok(changes) = &result {
                            let capture = cov_table.read().await.timed_capture_exact(changes);
                            capture.run(&db);
                        }
                        result
                    }
                }
            };

            match execution {
                Ok(result) => {
                    effects.life_safety_cov_changes.extend(result);
                    simple_ack(invoke_id, service_choice)
                }
                Err(e) => bacnet_device_core::reply::error_apdu(invoke_id, service_choice, &e),
            }
        }
    }
}
