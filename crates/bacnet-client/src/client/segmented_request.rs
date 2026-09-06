use super::*;
use std::future::{poll_fn, Future};
use std::task::Poll;

#[derive(Clone, Copy)]
pub(super) struct OutgoingSegmentContext<'a> {
    pub(super) target: ConfirmedTarget<'a>,
    pub(super) service_choice: ConfirmedServiceChoice,
    pub(super) advertised_max_apdu: u16,
    pub(super) remote_max_apdu: u16,
    pub(super) invoke_id: u8,
    pub(super) total_segments: usize,
    pub(super) tsm_mac: &'a MacAddr,
    pub(super) owner: &'a TransactionOwner,
}

pub(super) enum OutgoingSegmentSend {
    Sent,
    Terminal(TsmResponse),
    SegmentedResponse,
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    pub(super) async fn send_outgoing_segment(
        &self,
        context: OutgoingSegmentContext<'_>,
        seq: usize,
        segment_data: &Bytes,
        first_final_issue: bool,
        response_rx: &mut oneshot::Receiver<TsmResponse>,
        progress_rx: &mut tokio::sync::watch::Receiver<TransactionProgress>,
    ) -> Result<OutgoingSegmentSend, Error> {
        let is_last = seq == context.total_segments - 1;
        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: true,
            more_follows: !is_last,
            segmented_response_accepted: self.config.segmented_response_accepted,
            max_segments: self.config.max_segments,
            max_apdu_length: context.advertised_max_apdu,
            invoke_id: context.invoke_id,
            sequence_number: Some(seq as u8),
            proposed_window_size: Some(self.config.proposed_window_size.max(1)),
            service_choice: context.service_choice,
            service_request: segment_data.clone(),
        });
        let mut buf = BytesMut::with_capacity(context.remote_max_apdu as usize);
        encode_apdu(&mut buf, &pdu)?;
        if !self.routed_path_limits.authorize_attempt(
            context.target,
            context.tsm_mac,
            context.invoke_id,
            context.owner,
            buf.len(),
        ) {
            return response_rx
                .await
                .map(OutgoingSegmentSend::Terminal)
                .map_err(|_| Error::Encoding("TSM response channel closed".into()));
        }
        let send = self.send_confirmed_target_apdu(context.target, &buf);
        tokio::pin!(send);

        if first_final_issue {
            let Some(mut token) = self.tsm.lock().await.begin_final_segment_send(
                context.tsm_mac,
                context.invoke_id,
                context.owner,
            ) else {
                return response_rx
                    .await
                    .map(OutgoingSegmentSend::Terminal)
                    .map_err(|_| Error::Encoding("TSM response channel closed".into()));
            };

            // Poll without a TSM lock. Dispatch defers terminal admission while
            // the token is unresolved, so a response produced by this poll is
            // accepted only after SentAllSegments is published below.
            let first_poll = poll_fn(|cx| Poll::Ready(send.as_mut().poll(cx))).await;
            let send_completed = match first_poll {
                Poll::Ready(result) => {
                    result?;
                    true
                }
                Poll::Pending => false,
            };
            if !self.tsm.lock().await.mark_final_segment_issued(
                context.tsm_mac,
                context.invoke_id,
                context.owner,
                &mut token,
            ) {
                return response_rx
                    .await
                    .map(OutgoingSegmentSend::Terminal)
                    .map_err(|_| Error::Encoding("TSM response channel closed".into()));
            }
            if send_completed {
                return Ok(OutgoingSegmentSend::Sent);
            }
        }

        loop {
            tokio::select! {
                biased;
                response = &mut *response_rx => {
                    return response
                        .map(OutgoingSegmentSend::Terminal)
                        .map_err(|_| Error::Encoding("TSM response channel closed".into()));
                }
                changed = progress_rx.changed() => {
                    if changed.is_err() {
                        return response_rx
                            .await
                            .map(OutgoingSegmentSend::Terminal)
                            .map_err(|_| Error::Encoding("TSM response channel closed".into()));
                    }
                    if matches!(
                        *progress_rx.borrow_and_update(),
                        TransactionProgress::SegmentedResponse { .. }
                    ) {
                        return Ok(OutgoingSegmentSend::SegmentedResponse);
                    }
                }
                sent = &mut send => {
                    sent?;
                    return Ok(OutgoingSegmentSend::Sent);
                }
            }
        }
    }
}
