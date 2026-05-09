use std::sync::Arc;

use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, warn};

use crate::mstp_frame::{
    decode_frame, encode_frame, find_preamble, FrameType, MstpFrame, BROADCAST_MAC,
};
use crate::port::{ReceivedNpdu, TransportPort};

use super::{
    calculate_t_frame_abort_us, calculate_t_turnaround_us, next_addr, MasterNode, MasterState,
    MstpConfig, SerialPort, MSTP_MAX_FRAME_BUF, T_NO_TOKEN_MS, T_REPLY_DELAY_MS,
    T_REPLY_TIMEOUT_MS, T_USAGE_TIMEOUT_MS,
};

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
        let t_frame_abort_us = calculate_t_frame_abort_us(self.config.baud_rate);

        // Receive loop using tokio::select! with timer
        let task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            let mut frame_buf = Vec::with_capacity(2048);
            let mut last_byte_time = tokio::time::Instant::now();

            // Start with T_NO_TOKEN timeout — if we don't see anything, claim the token
            let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(T_NO_TOKEN_MS));
            tokio::pin!(sleep);

            let mut encode_buf = BytesMut::with_capacity(1024);

            loop {
                tokio::select! {
                    // Branch 1: serial data arrives
                    result = serial_clone.read(&mut recv_buf) => {
                        match result {
                            Ok(0) => continue,
                            Ok(n) => {
                                // T_frame_abort: discard partial frame if inter-byte gap
                                // exceeds the spec limit (60 bit times).
                                let now = tokio::time::Instant::now();
                                if !frame_buf.is_empty() {
                                    let gap = now.duration_since(last_byte_time);
                                    if gap > tokio::time::Duration::from_micros(t_frame_abort_us) {
                                        debug!(
                                            "MS/TP: T_frame_abort exceeded ({gap:?}), discarding partial frame"
                                        );
                                        frame_buf.clear();
                                    }
                                }
                                last_byte_time = now;

                                // Prevent unbounded growth from malformed input:
                                // check BEFORE extending to avoid a large allocation.
                                if frame_buf.len() + n > MSTP_MAX_FRAME_BUF {
                                    warn!(
                                        "MS/TP: frame buffer would overflow ({} + {} bytes), resetting",
                                        frame_buf.len(), n
                                    );
                                    frame_buf.clear();
                                    continue;
                                }
                                frame_buf.extend_from_slice(&recv_buf[..n]);
                            }
                            Err(e) => {
                                warn!("MS/TP serial read error: {}", e);
                                break;
                            }
                        }

                        // Try to find and decode frames
                        loop {
                            let preamble_pos = match find_preamble(&frame_buf) {
                                Some(pos) => pos,
                                None => {
                                    frame_buf.clear();
                                    break;
                                }
                            };

                            // Discard bytes before preamble
                            if preamble_pos > 0 {
                                frame_buf.drain(..preamble_pos);
                            }

                            match decode_frame(&frame_buf) {
                                Ok((frame, consumed)) => {
                                    frame_buf.drain(..consumed);

                                    // Process through state machine — collect
                                    // frames under lock, drop before writing.
                                    let mut node_guard = node.lock().await;
                                    let response =
                                        node_guard.handle_received_frame(&frame, &npdu_tx);
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
                                        // DoneWithToken: max_info_frames reached, pass token immediately
                                        if node_guard.state == MasterState::DoneWithToken {
                                            let token = node_guard.pass_token();
                                            encode_buf.clear();
                                            if let Err(e) = encode_frame(&mut encode_buf, &token) {
                                                warn!("MS/TP encode error: {}", e);
                                            } else {
                                                pending_writes.push(encode_buf.to_vec());
                                            }
                                            break;
                                        }
                                        let frame_to_send = node_guard.use_token();
                                        encode_buf.clear();
                                        if let Err(e) = encode_frame(&mut encode_buf, &frame_to_send) {
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
                                        // After sending Token, we're done
                                        if frame_to_send.frame_type == FrameType::Token {
                                            break;
                                        }
                                    }

                                    // Capture timeout before dropping lock
                                    let timeout_ms = match node_guard.state {
                                        MasterState::Idle => T_NO_TOKEN_MS,
                                        MasterState::NoToken => T_NO_TOKEN_MS,
                                        MasterState::PollForMaster => node_guard.t_slot_ms,
                                        MasterState::WaitForReply => T_REPLY_TIMEOUT_MS,
                                        MasterState::AnswerDataRequest => T_REPLY_DELAY_MS,
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
                                        tokio::time::Instant::now()
                                            + tokio::time::Duration::from_millis(timeout_ms),
                                    );
                                }
                                Err(_) => {
                                    // Incomplete frame or bad CRC — skip first preamble byte
                                    if frame_buf.len() > 2 {
                                        frame_buf.drain(..1);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    // Branch 2: timeout
                    () = &mut sleep => {
                        let mut node_guard = node.lock().await;
                        let mut pending_writes: Vec<Vec<u8>> = Vec::new();
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
                                // ReplyTimeout: enter DoneWithToken.
                                node_guard.expected_reply_source = None;
                                node_guard.frame_count = node_guard.config.max_info_frames;
                                node_guard.state = MasterState::DoneWithToken;
                                // Fall through to DoneWithToken handling on next iteration
                                T_USAGE_TIMEOUT_MS
                            }
                            MasterState::AnswerDataRequest => {
                                let dest = node_guard.pending_reply_source.unwrap_or(BROADCAST_MAC);
                                // Check if the application layer sent a reply via the channel.
                                let reply_data = node_guard.reply_rx.take().and_then(|mut rx| rx.try_recv().ok());
                                if let Some(data) = reply_data {
                                    // Send the reply as DataNotExpectingReply
                                    let reply_frame = MstpFrame {
                                        frame_type: FrameType::BACnetDataNotExpectingReply,
                                        destination: dest,
                                        source: node_guard.config.this_station,
                                        data,
                                    };
                                    encode_buf.clear();
                                    if let Ok(()) = encode_frame(&mut encode_buf, &reply_frame) {
                                        pending_writes.push(encode_buf.to_vec());
                                    }
                                } else {
                                    // No reply in time — send ReplyPostponed
                                    let rp = MstpFrame {
                                        frame_type: FrameType::ReplyPostponed,
                                        destination: dest,
                                        source: node_guard.config.this_station,
                                        data: Bytes::new(),
                                    };
                                    encode_buf.clear();
                                    if let Ok(()) = encode_frame(&mut encode_buf, &rp) {
                                        pending_writes.push(encode_buf.to_vec());
                                    }
                                }
                                node_guard.pending_reply_source = None;
                                node_guard.state = MasterState::Idle;
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
                                // Should not typically timeout in UseToken/DoneWithToken;
                                // pass the token and treat as idle
                                let token = node_guard.pass_token();
                                encode_buf.clear();
                                if let Ok(()) = encode_frame(&mut encode_buf, &token) {
                                    pending_writes.push(encode_buf.to_vec());
                                }
                                T_USAGE_TIMEOUT_MS
                            }
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
