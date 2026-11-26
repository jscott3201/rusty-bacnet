//! The wait for a confirmed request's answer (#2558), moved from
//! `requests.rs`, which was at its size cap. `wait_for_confirmed_response`
//! keeps the loop every request runs, the answer, a change of phase and the
//! timer; what a timer's expiry does is in a function per timer, which the
//! loop awaits only when that timer runs out: `request_timer_expired` for
//! AWAIT_CONFIRMATION (a retry, or the end of the wait) and
//! `segment_timer_expired` for SEGMENTED_CONF. Each is the match arm it
//! was, moved as it stood, except that it returns the phase to wait in
//! next where the arm set `progress`, or the wait's result where it
//! returned. Nothing else changed: the same locks, taken and released at
//! the same points, and no new await on the answer's path.
//!
//! The retry's log line moved with the code: its target is now
//! `bacnet_client::client::requests::wait`, which a filter on
//! `bacnet_client::client::requests` still covers.

use super::*;
use std::future::Future;

/// What the wait does after a timer ran out.
enum Expired {
    /// Wait on, in this phase.
    Wait(TransactionProgress),
    /// The wait ends with this.
    End(Result<TsmResponse, Error>),
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Wait through AWAIT_CONFIRMATION and SEGMENTED_CONF without allowing
    /// their timers to cancel each other's phase.
    pub(in crate::client) async fn wait_for_confirmed_response(
        &self,
        wait_for: ConfirmedWait<'_>,
        mut response_rx: oneshot::Receiver<TsmResponse>,
        mut progress_rx: ProgressReceiver,
    ) -> Result<TsmResponse, Error> {
        let (request_timeout, segment_timeout, max_retries) = {
            let tsm = self.tsm.lock();
            let config = tsm.config();
            (
                Duration::from_millis(config.apdu_timeout_ms),
                Duration::from_millis(config.apdu_segment_timeout_ms.saturating_mul(4)),
                config.apdu_retries,
            )
        };
        let mut retries_sent = 0u16;
        let mut progress = progress_rx.borrow_and_update();

        loop {
            let wait = match progress {
                TransactionProgress::AwaitingResponse => request_timeout,
                TransactionProgress::SegmentedResponse { .. } => segment_timeout,
            };
            let timer = tokio::time::sleep(wait);
            tokio::pin!(timer);

            tokio::select! {
                response = &mut response_rx => {
                    return response.map_err(|_| super::requests::cut_off(true));
                }
                // Progress is never published after the transaction leaves
                // the TSM; the response arm above resolves then instead.
                () = progress_rx.changed() => {
                    progress = progress_rx.borrow_and_update();
                }
                _ = &mut timer => {
                    let expired = match progress {
                        TransactionProgress::AwaitingResponse => {
                            self.request_timer_expired(
                                &wait_for,
                                (request_timeout, max_retries),
                                &mut retries_sent,
                                &mut response_rx,
                            )
                            .await
                        }
                        TransactionProgress::SegmentedResponse { generation } => {
                            self.segment_timer_expired(&wait_for, generation, &mut response_rx)
                                .await
                        }
                    };
                    match expired {
                        Expired::Wait(next) => progress = next,
                        Expired::End(result) => return result,
                    }
                }
            }
        }
    }

    /// The request timer ran out in AWAIT_CONFIRMATION: send the request
    /// again while retries are left, or end the wait. `timing` is the APDU
    /// timeout and the retries allowed; `retries_sent` counts the retries.
    ///
    /// Not an `async fn`, which would hold its moved arguments twice: the
    /// block below captures them once (#1713).
    #[allow(
        clippy::manual_async_fn,
        reason = "an async fn would hold its moved arguments twice (#1713)"
    )]
    fn request_timer_expired<'a>(
        &'a self,
        wait_for: &'a ConfirmedWait<'a>,
        timing: (Duration, u8),
        retries_sent: &'a mut u16,
        response_rx: &'a mut oneshot::Receiver<TsmResponse>,
    ) -> impl Future<Output = Expired> + Send + 'a {
        let (request_timeout, max_retries) = timing;
        async move {
            let ConfirmedWait {
                target,
                tsm_mac,
                invoke_id,
                owner,
                retry_apdu,
            } = *wait_for;
            let retry = retry_apdu.is_some() && *retries_sent < u16::from(max_retries);
            // The expiry and, for a retry with nothing to resend,
            // the cancel are one step under the TSM lock.
            let expiration = {
                let mut tsm = self.tsm.lock();
                let expiration = tsm.expire_request_timer(tsm_mac, invoke_id, owner, !retry);
                if matches!(expiration, RequestTimerExpiration::Retry) && retry_apdu.is_none() {
                    tsm.cancel_transaction_for_owner(tsm_mac, invoke_id, owner);
                }
                expiration
            };
            match expiration {
                RequestTimerExpiration::Retry => {
                    // Authorization and segment admission use the TSM lock,
                    // but transport I/O must not. Once authorized, this retry
                    // may finish even if segment zero is admitted meanwhile.
                    let Some(retry_apdu) = retry_apdu else {
                        self.enqueue_transaction_cleanup(tsm_mac, invoke_id, owner, false, None);
                        return Expired::End(Err(Error::Timeout(request_timeout)));
                    };
                    *retries_sent += 1;
                    if !self.routed_path_limits.authorize_attempt(
                        target,
                        tsm_mac,
                        invoke_id,
                        owner,
                        retry_apdu.len(),
                    ) {
                        return Expired::End(
                            (&mut *response_rx)
                                .await
                                .map_err(|_| super::requests::cut_off(true)),
                        );
                    }
                    let send_result = self
                        .send_confirmed_target_apdu(target, retry_apdu, owner)
                        .await;
                    if let Err(error) = send_result {
                        let expiration = self
                            .tsm
                            .lock()
                            .expire_request_timer(tsm_mac, invoke_id, owner, true);
                        match expiration {
                            RequestTimerExpiration::SegmentedResponse { generation } => {
                                return Expired::Wait(TransactionProgress::SegmentedResponse {
                                    generation,
                                });
                            }
                            RequestTimerExpiration::NoTransaction => {
                                return Expired::End(
                                    (&mut *response_rx)
                                        .await
                                        .map_err(|_| super::requests::cut_off(true)),
                                );
                            }
                            RequestTimerExpiration::TimedOut => {
                                self.enqueue_transaction_cleanup(
                                    tsm_mac, invoke_id, owner, false, None,
                                );
                                return Expired::End(Err(super::requests::retry_refused(error)));
                            }
                            RequestTimerExpiration::Retry | RequestTimerExpiration::Withdrawn => {
                                unreachable!(
                                    "final retry-send failure disposition cannot authorize another retry"
                                )
                            }
                        }
                    }
                    debug!(
                        invoke_id,
                        attempt = *retries_sent,
                        max_retries,
                        "APDU timeout, retrying confirmed request"
                    );
                    Expired::Wait(TransactionProgress::AwaitingResponse)
                }
                RequestTimerExpiration::SegmentedResponse { generation } => {
                    Expired::Wait(TransactionProgress::SegmentedResponse { generation })
                }
                // DeviceCommunicationControl restricts the
                // node: no retry goes out (#2617).
                RequestTimerExpiration::Withdrawn => {
                    self.enqueue_transaction_cleanup(tsm_mac, invoke_id, owner, false, None);
                    Expired::End(Err(Error::InitiationDisabled { sent: true }))
                }
                RequestTimerExpiration::TimedOut => {
                    self.enqueue_transaction_cleanup(tsm_mac, invoke_id, owner, false, None);
                    Expired::End(Err(Error::abort(
                        bacnet_types::enums::AbortReason::TSM_TIMEOUT,
                    )))
                }
                RequestTimerExpiration::NoTransaction => Expired::End(
                    (&mut *response_rx)
                        .await
                        .map_err(|_| super::requests::cut_off(true)),
                ),
            }
        }
    }

    /// The segment timer ran out in SEGMENTED_CONF: wait on if a segment
    /// came since `generation` or the transaction went back to
    /// AWAIT_CONFIRMATION, or end the wait.
    #[allow(
        clippy::manual_async_fn,
        reason = "an async fn would hold its moved arguments twice (#1713)"
    )]
    fn segment_timer_expired<'a>(
        &'a self,
        wait_for: &'a ConfirmedWait<'a>,
        generation: u64,
        response_rx: &'a mut oneshot::Receiver<TsmResponse>,
    ) -> impl Future<Output = Expired> + Send + 'a {
        async move {
            let ConfirmedWait {
                tsm_mac,
                invoke_id,
                owner,
                ..
            } = *wait_for;
            let expiration = self
                .tsm
                .lock()
                .expire_segment_timer(tsm_mac, invoke_id, owner, generation);
            match expiration {
                SegmentTimerExpiration::Activity { generation } => {
                    Expired::Wait(TransactionProgress::SegmentedResponse { generation })
                }
                SegmentTimerExpiration::AwaitingResponse => {
                    Expired::Wait(TransactionProgress::AwaitingResponse)
                }
                SegmentTimerExpiration::TimedOut => {
                    #[cfg(test)]
                    self.segmented_cleanup.pause_if_enabled().await;
                    self.enqueue_transaction_cleanup(tsm_mac, invoke_id, owner, false, None);
                    Expired::End(Err(Error::abort(
                        bacnet_types::enums::AbortReason::TSM_TIMEOUT,
                    )))
                }
                SegmentTimerExpiration::NoTransaction => Expired::End(
                    (&mut *response_rx)
                        .await
                        .map_err(|_| super::requests::cut_off(true)),
                ),
            }
        }
    }
}

#[cfg(test)]
#[path = "request_wait_tests.rs"]
mod tests;
