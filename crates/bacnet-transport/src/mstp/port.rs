use std::sync::Arc;

use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, warn};

use crate::mstp_frame::{
    decode_frame_stream, encode_frame, find_preamble, retain_lone_preamble_byte, FrameType,
    MstpFrame, StreamDecode, BROADCAST_MAC,
};
use crate::port::{ReceivedNpdu, TransportPort};

use super::{
    calculate_host_stale_partial_timeout_us, calculate_t_turnaround_us, next_addr, MasterNode,
    MasterState, MstpConfig, SerialPort, MSTP_MAX_FRAME_BUF, T_NO_TOKEN_MS, T_REPLY_DELAY_MS,
    T_REPLY_TIMEOUT_MS, T_REPLY_TRANSMIT_MARGIN_MS, T_USAGE_TIMEOUT_MS,
};

/// Add one host read to the persistent receive buffer and drain all complete frames.
///
/// The persistent buffer never exceeds one maximum standard frame. A host read may
/// contain any number of coalesced frames; complete frames are drained before more
/// bytes from that same read are appended.
fn assemble_host_chunk(frame_buf: &mut Vec<u8>, chunk: &[u8]) -> Vec<MstpFrame> {
    let mut frames = Vec::new();
    let mut remaining = chunk;

    while !remaining.is_empty() {
        let available = MSTP_MAX_FRAME_BUF.saturating_sub(frame_buf.len());
        if available == 0 {
            // A valid standard frame is decidable at this size. This fallback
            // guarantees progress if malformed input somehow remains NeedMore.
            warn!("MS/TP: full incomplete host assembly, discarding one byte");
            frame_buf.drain(..1);
            continue;
        }

        let take = available.min(remaining.len());
        frame_buf.extend_from_slice(&remaining[..take]);
        remaining = &remaining[take..];

        loop {
            let preamble_pos = match find_preamble(frame_buf) {
                Some(pos) => pos,
                None => {
                    retain_lone_preamble_byte(frame_buf);
                    break;
                }
            };

            if preamble_pos > 0 {
                frame_buf.drain(..preamble_pos);
            }

            match decode_frame_stream(frame_buf) {
                StreamDecode::Complete { frame, consumed } => {
                    frame_buf.drain(..consumed);
                    frames.push(frame);
                }
                StreamDecode::NeedMore => break,
                StreamDecode::Invalid { discard } => {
                    let discard = discard.min(frame_buf.len()).max(1);
                    frame_buf.drain(..discard);
                }
            }
        }
    }

    debug_assert!(frame_buf.len() <= MSTP_MAX_FRAME_BUF);
    frames
}

fn finish_data_request_at_transport_boundary(
    node: &mut MasterNode,
    reply_data: Option<Bytes>,
) -> Option<MstpFrame> {
    match node.finish_data_request(reply_data) {
        Ok(frame) => Some(frame),
        Err(error) => {
            warn!("MS/TP application reply rejected: {error}");
            node.abandon_data_request();
            None
        }
    }
}

// ---------------------------------------------------------------------------
// MS/TP Transport
// ---------------------------------------------------------------------------

/// MS/TP transport implementing [`TransportPort`].
pub struct MstpTransport<S: SerialPort> {
    serial: Option<S>,
    config: MstpConfig,
    local_mac: [u8; 1],
    node: Option<Arc<Mutex<MasterNode>>>,
    recv_task: Option<tokio::task::JoinHandle<()>>,
}

impl<S: SerialPort> MstpTransport<S> {
    pub fn new(serial: S, config: MstpConfig) -> Self {
        let mac = config.this_station;
        Self {
            serial: Some(serial),
            config,
            local_mac: [mac],
            node: None,
            recv_task: None,
        }
    }

    /// Get the master node state (for testing/inspection).
    pub fn node_state(&self) -> Option<&Arc<Mutex<MasterNode>>> {
        self.node.as_ref()
    }

    fn abort_receive_task_and_release_state(&mut self) {
        if let Some(task) = self.recv_task.take() {
            task.abort();
        }
        self.serial.take();
        self.node.take();
    }
}

impl<S: SerialPort> TransportPort for MstpTransport<S> {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        /// NPDU receive channel capacity — smaller than BIP/Ethernet for low-bandwidth serial.
        const NPDU_CHANNEL_CAPACITY: usize = 64;

        let (npdu_tx, npdu_rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);

        let node = Arc::new(Mutex::new(MasterNode::new(self.config.clone())?));
        self.node = Some(node.clone());

        let serial = self
            .serial
            .take()
            .ok_or_else(|| Error::Encoding("MS/TP transport already started".into()))?;

        let serial = Arc::new(serial);
        let serial_clone = serial.clone();
        let t_turnaround_us = calculate_t_turnaround_us(self.config.baud_rate);
        // Host reassembly policy: tolerate USB read chunk gaps, not wire T_frame_abort.
        let host_stale_partial_timeout_us =
            calculate_host_stale_partial_timeout_us(self.config.baud_rate);
        let reply_decision_delay_ms = T_REPLY_DELAY_MS
            .saturating_sub(t_turnaround_us.div_ceil(1_000) + T_REPLY_TRANSMIT_MARGIN_MS);

        // Receive loop using tokio::select! with timer
        let task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            let mut frame_buf = Vec::with_capacity(MSTP_MAX_FRAME_BUF);
            let mut last_byte_time = tokio::time::Instant::now();

            // Start with T_NO_TOKEN timeout — if we don't see anything, claim the token
            let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(T_NO_TOKEN_MS));
            tokio::pin!(sleep);

            let mut encode_buf = BytesMut::with_capacity(1024);
            let mut pending_reply_rx: Option<oneshot::Receiver<Bytes>> = None;
            let mut pending_reply_deadline: Option<tokio::time::Instant> = None;

            loop {
                tokio::select! {
                    biased;
                    // A DataExpectingReply application response must wake the
                    // loop immediately; waiting for T_reply_delay would begin
                    // transmission after the Standard's deadline.
                    reply = async {
                        pending_reply_rx
                            .as_mut()
                            .expect("reply branch is guarded")
                            .await
                    }, if pending_reply_rx.is_some() => {
                        pending_reply_rx = None;
                        pending_reply_deadline = None;
                        let mut node_guard = node.lock().await;
                        let response = finish_data_request_at_transport_boundary(
                            &mut node_guard,
                            reply.ok(),
                        );
                        drop(node_guard);

                        if let Some(response) = response {
                            encode_buf.clear();
                            if encode_frame(&mut encode_buf, &response).is_ok() {
                                tokio::time::sleep(tokio::time::Duration::from_micros(
                                    t_turnaround_us,
                                ))
                                .await;
                                if let Err(e) = serial_clone.write(&encode_buf).await {
                                    warn!("MS/TP write error: {}", e);
                                }
                            }
                        }
                        sleep.as_mut().reset(
                            tokio::time::Instant::now()
                                + tokio::time::Duration::from_millis(T_USAGE_TIMEOUT_MS),
                        );
                    }
                    // Branch 1: serial data arrives
                    result = serial_clone.read(&mut recv_buf) => {
                        let frames = match result {
                            Ok(0) => continue,
                            Ok(n) => {
                                // Host stale-partial timeout: drop abandoned assembly if no
                                // bytes arrive for a long host-side gap (USB scheduling, not
                                // Clause 9 wire T_frame_abort).
                                let now = tokio::time::Instant::now();
                                if !frame_buf.is_empty() {
                                    let gap = now.duration_since(last_byte_time);
                                    if gap
                                        > tokio::time::Duration::from_micros(
                                            host_stale_partial_timeout_us,
                                        )
                                    {
                                        debug!(
                                            "MS/TP: host stale partial frame timeout ({gap:?}), discarding partial assembly"
                                        );
                                        frame_buf.clear();
                                    }
                                }
                                last_byte_time = now;
                                assemble_host_chunk(&mut frame_buf, &recv_buf[..n])
                            }
                            Err(e) => {
                                warn!("MS/TP serial read error: {}", e);
                                break;
                            }
                        };

                        for frame in frames {
                                    // Process through state machine — collect
                                    // frames under lock, drop before writing.
                                    let mut node_guard = node.lock().await;
                                    let response =
                                        node_guard.handle_received_frame(&frame, &npdu_tx);
                                    let mut started_reply = false;
                                    if pending_reply_rx.is_none()
                                        && node_guard.state == MasterState::AnswerDataRequest
                                    {
                                        pending_reply_rx = node_guard.reply_rx.take();
                                        started_reply = pending_reply_rx.is_some();
                                    }
                                    if started_reply {
                                        pending_reply_deadline = Some(
                                            last_byte_time
                                                + tokio::time::Duration::from_millis(
                                                    reply_decision_delay_ms,
                                                ),
                                        );
                                    }
                                    let mut pending_writes: Vec<Vec<u8>> = Vec::new();
                                    if let Some(response) = response {
                                        encode_buf.clear();
                                        if let Err(e) = encode_frame(&mut encode_buf, &response) {
                                            warn!("MS/TP encode error: {}", e);
                                            drop(node_guard);
                                            continue;
                                        }
                                        pending_writes.push(encode_buf.to_vec());
                                    }

                                    // If we got the token, use it
                                    while node_guard.state == MasterState::UseToken
                                        || node_guard.state == MasterState::DoneWithToken
                                    {
                                        let frame_to_send =
                                            if node_guard.state == MasterState::DoneWithToken {
                                                node_guard.done_with_token()
                                            } else {
                                                node_guard.use_token()
                                            };
                                        encode_buf.clear();
                                        if let Err(e) = encode_frame(&mut encode_buf, &frame_to_send)
                                        {
                                            warn!("MS/TP encode error: {}", e);
                                            break;
                                        }
                                        pending_writes.push(encode_buf.to_vec());
                                        // After sending DataExpectingReply, enter WaitForReply
                                        if frame_to_send.frame_type
                                            == FrameType::BACnetDataExpectingReply
                                        {
                                            node_guard.state = MasterState::WaitForReply;
                                            break;
                                        }
                                        // After Token or PFM, leave the use/done loop
                                        if matches!(
                                            frame_to_send.frame_type,
                                            FrameType::Token | FrameType::PollForMaster
                                        ) {
                                            break;
                                        }
                                        // SoleMaster may return to UseToken with no wire
                                        // progress; break if still UseToken after empty cycle
                                        if node_guard.state == MasterState::UseToken
                                            && node_guard.tx_queue.is_empty()
                                            && node_guard.frame_count
                                                >= node_guard.config.max_info_frames
                                        {
                                            break;
                                        }
                                    }

                                    // Capture timeout before dropping lock
                                    let timeout_ms = match node_guard.state {
                                        MasterState::Idle => T_NO_TOKEN_MS,
                                        MasterState::NoToken => T_NO_TOKEN_MS,
                                        MasterState::PollForMaster => node_guard.t_slot_ms,
                                        MasterState::WaitForReply => T_REPLY_TIMEOUT_MS,
                                        MasterState::AnswerDataRequest => reply_decision_delay_ms,
                                        MasterState::PassToken => T_USAGE_TIMEOUT_MS,
                                        MasterState::UseToken
                                        | MasterState::DoneWithToken => T_USAGE_TIMEOUT_MS,
                                    };
                                    drop(node_guard);

                                    // T_turnaround before transmitting
                                    if !pending_writes.is_empty() {
                                        tokio::time::sleep(tokio::time::Duration::from_micros(
                                            t_turnaround_us,
                                        ))
                                        .await;
                                    }
                                    for frame_data in &pending_writes {
                                        if let Err(e) = serial_clone.write(frame_data).await {
                                            warn!("MS/TP write error: {}", e);
                                            break;
                                        }
                                    }

                                    sleep.as_mut().reset(
                                        pending_reply_deadline.unwrap_or_else(|| {
                                            tokio::time::Instant::now()
                                                + tokio::time::Duration::from_millis(timeout_ms)
                                        }),
                                    );
                        }
                    }
                    // Branch 2: timeout
                    () = &mut sleep => {
                        let mut node_guard = node.lock().await;
                        let mut pending_writes: Vec<Vec<u8>> = Vec::new();
                        let was_answering_data_request =
                            node_guard.pending_reply_source.is_some();
                        let timeout_ms = match node_guard.state {
                            MasterState::Idle => {
                                node_guard.state = MasterState::NoToken;
                                node_guard.retry_token_count = 0;
                                let ts = node_guard.config.this_station as u64;
                                T_NO_TOKEN_MS + node_guard.t_slot_ms * ts
                            }
                            MasterState::NoToken => {
                                // GenerateToken: send PFM to discover successor.
                                let ts = node_guard.config.this_station;
                                let pfm = MstpFrame {
                                    frame_type: FrameType::PollForMaster,
                                    destination: next_addr(ts, node_guard.config.max_master),
                                    source: ts,
                                    data: Bytes::new(),
                                };
                                encode_buf.clear();
                                if let Ok(()) = encode_frame(&mut encode_buf, &pfm) {
                                    pending_writes.push(encode_buf.to_vec());
                                }
                                node_guard.poll_station =
                                    next_addr(ts, node_guard.config.max_master);
                                node_guard.state = MasterState::PollForMaster;
                                node_guard.poll_count = 0;
                                node_guard.t_slot_ms
                            }
                            MasterState::PollForMaster => {
                                // No reply to PFM — try next
                                let frame_to_send = node_guard.poll_timeout();
                                encode_buf.clear();
                                if let Ok(()) = encode_frame(&mut encode_buf, &frame_to_send) {
                                    pending_writes.push(encode_buf.to_vec());
                                }
                                if node_guard.state == MasterState::PollForMaster {
                                    node_guard.t_slot_ms
                                } else {
                                    T_USAGE_TIMEOUT_MS
                                }
                            }
                            MasterState::WaitForReply => {
                                // ReplyTimeout: enter DoneWithToken then run transitions.
                                node_guard.expected_reply_source = None;
                                node_guard.frame_count = node_guard.config.max_info_frames;
                                node_guard.state = MasterState::DoneWithToken;
                                let frame_to_send = node_guard.done_with_token();
                                encode_buf.clear();
                                if let Ok(()) = encode_frame(&mut encode_buf, &frame_to_send) {
                                    pending_writes.push(encode_buf.to_vec());
                                }
                                match node_guard.state {
                                    MasterState::PassToken => T_USAGE_TIMEOUT_MS,
                                    MasterState::PollForMaster => node_guard.t_slot_ms,
                                    MasterState::UseToken => T_USAGE_TIMEOUT_MS,
                                    _ => T_USAGE_TIMEOUT_MS,
                                }
                            }
                            MasterState::AnswerDataRequest => {
                                // The timer fires early enough to include
                                // turnaround and scheduling before the first
                                // octet's T_reply_delay deadline. Prefer a
                                // response that became ready at the boundary.
                                let reply_data = pending_reply_rx
                                    .take()
                                    .and_then(|mut rx| rx.try_recv().ok());
                                if let Some(reply_frame) = finish_data_request_at_transport_boundary(
                                    &mut node_guard,
                                    reply_data,
                                ) {
                                    encode_buf.clear();
                                    if encode_frame(&mut encode_buf, &reply_frame).is_ok() {
                                        pending_writes.push(encode_buf.to_vec());
                                    }
                                }
                                T_USAGE_TIMEOUT_MS
                            }
                            MasterState::PassToken => {
                                if let Some(frame) = node_guard.pass_token_timeout() {
                                    encode_buf.clear();
                                    if let Ok(()) = encode_frame(&mut encode_buf, &frame) {
                                        pending_writes.push(encode_buf.to_vec());
                                    }
                                }
                                match node_guard.state {
                                    MasterState::PassToken => T_USAGE_TIMEOUT_MS,
                                    MasterState::NoToken => {
                                        // Per spec Clause 9.5.6: T_no_token + T_slot * TS
                                        let ts = node_guard.config.this_station as u64;
                                        T_NO_TOKEN_MS + node_guard.t_slot_ms * ts
                                    }
                                    MasterState::UseToken => T_USAGE_TIMEOUT_MS,
                                    _ => T_USAGE_TIMEOUT_MS,
                                }
                            }
                            MasterState::UseToken
                            | MasterState::DoneWithToken => {
                                // Should not typically timeout here; run DONE_WITH_TOKEN
                                // transitions (never unconditional pass_token).
                                node_guard.state = MasterState::DoneWithToken;
                                let frame_to_send = node_guard.done_with_token();
                                encode_buf.clear();
                                if let Ok(()) = encode_frame(&mut encode_buf, &frame_to_send) {
                                    pending_writes.push(encode_buf.to_vec());
                                }
                                match node_guard.state {
                                    MasterState::PassToken => T_USAGE_TIMEOUT_MS,
                                    MasterState::PollForMaster => node_guard.t_slot_ms,
                                    MasterState::UseToken => T_USAGE_TIMEOUT_MS,
                                    _ => T_USAGE_TIMEOUT_MS,
                                }
                            }
                        };
                        if was_answering_data_request {
                            pending_reply_deadline = None;
                        }
                        drop(node_guard);

                        // T_turnaround before transmitting
                        if !pending_writes.is_empty() {
                            tokio::time::sleep(tokio::time::Duration::from_micros(
                                t_turnaround_us,
                            ))
                            .await;
                        }
                        for frame_data in &pending_writes {
                            if let Err(e) = serial_clone.write(frame_data).await {
                                warn!("MS/TP write error: {}", e);
                                break;
                            }
                        }

                        sleep.as_mut().reset(
                            tokio::time::Instant::now()
                                + tokio::time::Duration::from_millis(timeout_ms),
                        );
                    }
                }
            }
        });

        self.recv_task = Some(task);
        Ok(npdu_rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.recv_task.take() {
            task.abort();
            let _ = task.await;
        }
        // Clear the node's queue to prevent stale sends after stop
        if let Some(ref node) = self.node {
            let mut n = node.lock().await;
            n.tx_queue.clear();
            n.state = MasterState::Idle;
            n.expected_reply_source = None;
        }
        Ok(())
    }

    fn abort(&mut self) {
        self.abort_receive_task_and_release_state();
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        if mac.len() != 1 {
            return Err(Error::Encoding(format!(
                "MS/TP MAC must be 1 byte, got {}",
                mac.len()
            )));
        }
        let dest = mac[0];
        if let Some(ref node) = self.node {
            let mut node = node.lock().await;
            node.queue_npdu(dest, Bytes::copy_from_slice(npdu))?;
            Ok(())
        } else {
            Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "MS/TP transport not started",
            )))
        }
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        if let Some(ref node) = self.node {
            let mut node = node.lock().await;
            node.queue_npdu(BROADCAST_MAC, Bytes::copy_from_slice(npdu))?;
            Ok(())
        } else {
            Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "MS/TP transport not started",
            )))
        }
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    fn max_apdu_length(&self) -> u16 {
        480
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        mac == [BROADCAST_MAC]
    }
}

impl<S: SerialPort> Drop for MstpTransport<S> {
    fn drop(&mut self) {
        self.abort_receive_task_and_release_state();
    }
}

// ---------------------------------------------------------------------------
// Loopback serial port for testing
// ---------------------------------------------------------------------------

/// In-memory loopback serial port for unit testing.
pub struct LoopbackSerial {
    rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    tx: mpsc::Sender<Vec<u8>>,
    /// Leftover bytes from a previous read that didn't fit in the caller's buffer.
    leftover: Mutex<Vec<u8>>,
}

impl LoopbackSerial {
    /// Create a pair of connected loopback serial ports.
    pub fn pair() -> (Self, Self) {
        let (tx_a, rx_b) = mpsc::channel(64);
        let (tx_b, rx_a) = mpsc::channel(64);
        (
            Self {
                rx: Mutex::new(rx_a),
                tx: tx_a,
                leftover: Mutex::new(Vec::new()),
            },
            Self {
                rx: Mutex::new(rx_b),
                tx: tx_b,
                leftover: Mutex::new(Vec::new()),
            },
        )
    }
}

impl SerialPort for LoopbackSerial {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        self.tx
            .send(data.to_vec())
            .await
            .map_err(|_| Error::Encoding("loopback write failed".into()))
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        // Serve leftover bytes first
        let mut leftover = self.leftover.lock().await;
        if !leftover.is_empty() {
            let len = leftover.len().min(buf.len());
            buf[..len].copy_from_slice(&leftover[..len]);
            leftover.drain(..len);
            return Ok(len);
        }
        drop(leftover);

        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(data) => {
                let len = data.len().min(buf.len());
                buf[..len].copy_from_slice(&data[..len]);
                // Buffer excess bytes for next read
                if data.len() > buf.len() {
                    let mut leftover = self.leftover.lock().await;
                    leftover.extend_from_slice(&data[buf.len()..]);
                }
                Ok(len)
            }
            None => Err(Error::Encoding("loopback channel closed".into())),
        }
    }
}

// ---------------------------------------------------------------------------
// NoSerial: zero-sized SerialPort for non-MS/TP contexts
// ---------------------------------------------------------------------------

/// A serial port implementation that always errors.
///
/// Used to satisfy the `AnyTransport<S>` generic when MS/TP is not needed
/// (e.g., in Python bindings where serial access isn't exposed).
pub struct NoSerial;

impl SerialPort for NoSerial {
    async fn write(&self, _data: &[u8]) -> Result<(), Error> {
        Err(Error::Encoding("NoSerial: MS/TP not available".into()))
    }

    async fn read(&self, _buf: &mut [u8]) -> Result<usize, Error> {
        Err(Error::Encoding("NoSerial: MS/TP not available".into()))
    }
}

#[cfg(test)]
mod assembly_tests {
    use super::*;
    use crate::mstp_frame::{MAX_STANDARD_MPDU_DATA, PREAMBLE};

    fn encode_host_data_frame(source: u8, fill: u8, data_len: usize) -> Vec<u8> {
        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataNotExpectingReply,
            destination: 3,
            source,
            data: Bytes::from(vec![fill; data_len]),
        };
        let mut wire = BytesMut::new();
        encode_frame(&mut wire, &frame).unwrap();
        wire.to_vec()
    }

    #[test]
    fn drains_coalesced_frames_larger_than_one_frame() {
        let first = encode_host_data_frame(1, 0xA1, 300);
        let second = encode_host_data_frame(2, 0xB2, 300);
        let mut chunk = first;
        chunk.extend_from_slice(&second);
        assert!(chunk.len() > MSTP_MAX_FRAME_BUF);

        let mut frame_buf = Vec::new();
        let frames = assemble_host_chunk(&mut frame_buf, &chunk);

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].source, 1);
        assert_eq!(frames[0].data, Bytes::from(vec![0xA1; 300]));
        assert_eq!(frames[1].source, 2);
        assert_eq!(frames[1].data, Bytes::from(vec![0xB2; 300]));
        assert!(frame_buf.is_empty());
    }

    #[test]
    fn bounds_malformed_input_and_retains_max_partial() {
        let malformed = vec![0xAA; MSTP_MAX_FRAME_BUF * 4 + 17];
        let mut frame_buf = Vec::new();
        assert!(assemble_host_chunk(&mut frame_buf, &malformed).is_empty());
        assert!(frame_buf.len() <= MSTP_MAX_FRAME_BUF);

        let wire = encode_host_data_frame(1, 0xCC, MAX_STANDARD_MPDU_DATA);
        assert_eq!(wire.len(), MSTP_MAX_FRAME_BUF);
        let split = wire.len() - 1;
        assert!(assemble_host_chunk(&mut frame_buf, &wire[..split]).is_empty());
        assert_eq!(frame_buf.len(), split);

        let frames = assemble_host_chunk(&mut frame_buf, &wire[split..]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data.len(), MAX_STANDARD_MPDU_DATA);
        assert!(frame_buf.is_empty());

        let token = MstpFrame {
            frame_type: FrameType::Token,
            destination: 3,
            source: 1,
            data: Bytes::new(),
        };
        let mut token_wire = BytesMut::new();
        encode_frame(&mut token_wire, &token).unwrap();
        assert!(assemble_host_chunk(&mut frame_buf, &[0xAA, PREAMBLE[0]]).is_empty());
        assert_eq!(frame_buf, PREAMBLE[..1]);
        let frames = assemble_host_chunk(&mut frame_buf, &token_wire[1..]);
        assert_eq!(frames, vec![token]);
        assert!(frame_buf.is_empty());
    }
}

#[cfg(test)]
mod abort_tests {
    use std::future::pending;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::*;

    struct ExclusiveSerial(Arc<AtomicBool>);

    impl ExclusiveSerial {
        fn open(in_use: Arc<AtomicBool>) -> Option<Self> {
            in_use
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .ok()
                .map(|_| Self(in_use))
        }
    }

    impl Drop for ExclusiveSerial {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    impl SerialPort for ExclusiveSerial {
        async fn write(&self, _data: &[u8]) -> Result<(), Error> {
            Ok(())
        }

        async fn read(&self, _buf: &mut [u8]) -> Result<usize, Error> {
            pending().await
        }
    }

    async fn reopen(in_use: &Arc<AtomicBool>) -> ExclusiveSerial {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(serial) = ExclusiveSerial::open(in_use.clone()) {
                    break serial;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("aborted receive task retained the serial resource")
    }

    #[tokio::test]
    async fn abort_takes_task_and_releases_serial_for_reopen() {
        let in_use = Arc::new(AtomicBool::new(false));
        let serial = ExclusiveSerial::open(in_use.clone()).unwrap();
        let mut transport = MstpTransport::new(serial, MstpConfig::default());
        let _rx = transport.start().await.unwrap();
        assert!(ExclusiveSerial::open(in_use.clone()).is_none());

        transport.abort();
        assert!(transport.recv_task.is_none());
        assert!(transport.node.is_none());
        drop(reopen(&in_use).await);
        assert!(!in_use.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn drop_aborts_task_and_releases_serial_for_reopen() {
        let in_use = Arc::new(AtomicBool::new(false));
        let serial = ExclusiveSerial::open(in_use.clone()).unwrap();
        let mut transport = MstpTransport::new(serial, MstpConfig::default());
        let _rx = transport.start().await.unwrap();

        drop(transport);
        drop(reopen(&in_use).await);
        assert!(!in_use.load(Ordering::SeqCst));
    }
}
