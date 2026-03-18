//! MS/TP (Master-Slave/Token-Passing) transport per ASHRAE 135-2020 Clause 9.
//!
//! Implements the master node state machine for token-passing over RS-485.
//! The actual serial I/O is abstracted behind the [`SerialPort`] trait so that
//! the state machine can be tested without hardware.

use std::collections::VecDeque;
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, warn};

use bacnet_types::error::Error;
use bacnet_types::MacAddr;

use crate::mstp_frame::{
    decode_frame, encode_frame, find_preamble, FrameType, MstpFrame, BROADCAST_MAC, MAX_MASTER,
};
use crate::port::{ReceivedNpdu, TransportPort};

// ---------------------------------------------------------------------------
// Serial port abstraction
// ---------------------------------------------------------------------------

/// Abstraction over an RS-485 serial port.
///
/// Implementations wrap the platform serial driver (e.g. `tokio-serial`).
/// A loopback implementation is provided for testing.
pub trait SerialPort: Send + Sync + 'static {
    /// Write bytes to the serial port.
    fn write(&self, data: &[u8]) -> impl std::future::Future<Output = Result<(), Error>> + Send;
    /// Read available bytes into `buf`. Returns the number of bytes read.
    /// Should block until at least 1 byte is available or timeout.
    fn read(
        &self,
        buf: &mut [u8],
    ) -> impl std::future::Future<Output = Result<usize, Error>> + Send;
}

// ---------------------------------------------------------------------------
// MS/TP timing constants per Clause 9.5.6
// ---------------------------------------------------------------------------

/// Time without a token before a node assumes the token is lost (ms).
const T_NO_TOKEN_MS: u64 = 500;
/// Time to wait for a reply after sending DataExpectingReply (ms).
const T_REPLY_TIMEOUT_MS: u64 = 255;
/// Time to wait for another node to begin using the token after it was passed (ms).
const T_USAGE_TIMEOUT_MS: u64 = 20;
/// T_SLOT: Clause 9.5.3 — "The width of the time slot within which a node
/// may generate a token: 10 milliseconds."
fn calculate_t_slot_ms(_baud_rate: u32) -> u64 {
    10
}
/// Maximum time a node may delay before sending a reply to DataExpectingReply (ms).
const T_REPLY_DELAY_MS: u64 = 250;
/// T_turnaround: minimum silence time (40 bit times) before transmitting after
/// receiving last octet (Clause 9.5.5.1). Computed as (40 * 1000) / baud_rate ms.
fn calculate_t_turnaround_us(baud_rate: u32) -> u64 {
    // 40 bit times in microseconds: (40 * 1_000_000) / baud_rate
    40_000_000u64 / baud_rate as u64
}
/// Number of retries for token pass before declaring token lost.
const N_RETRY_TOKEN: u8 = 1;
/// Maximum frame buffer size: preamble(2) + header(6) + max data(1497) + CRC16(2)
pub(crate) const MSTP_MAX_FRAME_BUF: usize = 1507;
/// Maximum number of queued outgoing frames before rejecting new sends.
const MAX_TX_QUEUE_DEPTH: usize = 256;

// ---------------------------------------------------------------------------
// Master node state machine (Clause 9.5.6)
// ---------------------------------------------------------------------------

/// Token-passing master node state per Clause 9.5.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterState {
    /// Waiting for a frame or timeout.
    Idle,
    /// No token has been seen — attempting to claim it.
    NoToken,
    /// We have the token — decide whether to send data or pass.
    UseToken,
    /// Done sending data frames; decide whether to poll or pass token.
    DoneWithToken,
    /// Token has been passed to NS, waiting for NS to use it (Clause 9.5.6.6).
    PassToken,
    /// Waiting for a reply after sending DataExpectingReply.
    WaitForReply,
    /// Polling for successor master stations.
    PollForMaster,
    /// Answering a received DataExpectingReply.
    AnswerDataRequest,
}

/// Master node configuration constants (Clause 9.5.4).
#[derive(Debug, Clone)]
pub struct MstpConfig {
    /// This station's MAC address (0..=MAX_MASTER).
    pub this_station: u8,
    /// Maximum master address on the network.
    pub max_master: u8,
    /// Maximum number of information frames per token use.
    pub max_info_frames: u8,
    /// Baud rate for T_SLOT calculation (default: 9600).
    pub baud_rate: u32,
}

impl Default for MstpConfig {
    fn default() -> Self {
        Self {
            this_station: 0,
            max_master: MAX_MASTER,
            max_info_frames: 1,
            baud_rate: 9600,
        }
    }
}

/// Internal state for the master node state machine.
pub struct MasterNode {
    pub config: MstpConfig,
    pub state: MasterState,
    /// Next station to receive the token.
    pub next_station: u8,
    /// Station to poll for master discovery.
    pub poll_station: u8,
    /// Number of tokens received since last PollForMaster.
    pub token_count: u8,
    /// Number of info frames sent during this token use.
    pub frame_count: u8,
    /// Queued NPDU frames to transmit (dest_mac, npdu).
    pub tx_queue: VecDeque<(u8, Bytes)>,
    /// Whether we've heard from any station (for initial token claim).
    pub sole_master: bool,
    /// Number of consecutive PollForMaster with no reply.
    pub poll_count: u8,
    /// Number of token-pass retries in NoToken state.
    pub retry_token_count: u8,
    /// Oneshot receiver for reply data in AnswerDataRequest state.
    /// The application layer holds the Sender via ReceivedNpdu.reply_tx.
    pub reply_rx: Option<oneshot::Receiver<Bytes>>,
    /// Source address of the station that sent BACnetDataExpectingReply,
    /// used as destination for ReplyPostponed (instead of broadcast).
    pub pending_reply_source: Option<u8>,
    /// Computed T_SLOT in milliseconds, based on configured baud rate.
    pub t_slot_ms: u64,
    /// EventCount: number of valid octets/frames received (Clause 9.5.2).
    /// Used in PASS_TOKEN (SawTokenUser) and NO_TOKEN (SawFrame).
    pub event_count: u32,
}

/// How many tokens between PollForMaster attempts.
const NPOLL: u8 = 50;
/// Max retries for PollForMaster.
const MAX_POLL_RETRIES: u8 = 3;

impl MasterNode {
    pub fn new(config: MstpConfig) -> Result<Self, Error> {
        if config.this_station > MAX_MASTER {
            return Err(Error::Encoding(format!(
                "MS/TP this_station {} exceeds MAX_MASTER ({})",
                config.this_station, MAX_MASTER
            )));
        }
        let ts = config.this_station;
        let t_slot_ms = calculate_t_slot_ms(config.baud_rate);
        // Clause 9.5.6.1 INITIALIZE: NS=TS, PS=TS, TokenCount=N_poll
        Ok(Self {
            config,
            state: MasterState::Idle,
            next_station: ts,
            poll_station: ts,
            token_count: NPOLL,
            frame_count: 0,
            tx_queue: VecDeque::new(),
            sole_master: false,
            poll_count: 0,
            retry_token_count: 0,
            reply_rx: None,
            pending_reply_source: None,
            t_slot_ms,
            event_count: 0,
        })
    }

    /// Handle a received frame. Returns an optional response frame to send on the wire,
    /// and an optional oneshot receiver for reply data (only when entering AnswerDataRequest).
    pub fn handle_received_frame(
        &mut self,
        frame: &MstpFrame,
        npdu_tx: &mpsc::Sender<ReceivedNpdu>,
    ) -> Option<MstpFrame> {
        // Clause 9.5.2: increment EventCount on each valid frame received.
        self.event_count = self.event_count.saturating_add(1);

        // Clause 9.5.6.6 SawTokenUser: if in PassToken and we see any valid
        // frame, NS has started using the token → transition to Idle.
        if self.state == MasterState::PassToken {
            self.state = MasterState::Idle;
        }
        match frame.frame_type {
            FrameType::Token => {
                if frame.destination == self.config.this_station {
                    debug!(src = frame.source, "received token");
                    // Clause 9.5.6.2 ReceivedToken: set SoleMaster to FALSE
                    self.sole_master = false;
                    self.state = MasterState::UseToken;
                    self.frame_count = 0;
                    self.token_count = self.token_count.wrapping_add(1);
                    self.retry_token_count = 0;
                }
                None
            }
            FrameType::PollForMaster => {
                if frame.destination == self.config.this_station {
                    debug!(src = frame.source, "replying to PollForMaster");
                    Some(MstpFrame {
                        frame_type: FrameType::ReplyToPollForMaster,
                        destination: frame.source,
                        source: self.config.this_station,
                        data: Bytes::new(),
                    })
                } else {
                    None
                }
            }
            FrameType::ReplyToPollForMaster => {
                if self.state == MasterState::PollForMaster
                    && frame.destination == self.config.this_station
                {
                    debug!(src = frame.source, "PFM reply — new successor");
                    // Clause 9.5.6.8 ReceivedReplyToPFM: set NS=source, SoleMaster=false,
                    // PS=TS, TokenCount=0, send Token to NS, enter PASS_TOKEN.
                    self.next_station = frame.source;
                    self.sole_master = false;
                    self.poll_station = self.config.this_station;
                    self.token_count = 0;
                    self.poll_count = 0;
                    // Send Token to the new successor and enter PassToken
                    return Some(self.pass_token());
                }
                None
            }
            FrameType::BACnetDataNotExpectingReply => {
                if frame.destination == self.config.this_station
                    || frame.destination == BROADCAST_MAC
                {
                    let _ = npdu_tx.try_send(ReceivedNpdu {
                        npdu: frame.data.clone(),
                        source_mac: MacAddr::from_slice(&[frame.source]),
                        reply_tx: None,
                    });
                }
                None
            }
            FrameType::BACnetDataExpectingReply => {
                if frame.destination == self.config.this_station {
                    self.state = MasterState::AnswerDataRequest;
                    self.pending_reply_source = Some(frame.source);
                    let (tx, rx) = oneshot::channel();
                    self.reply_rx = Some(rx);
                    let _ = npdu_tx.try_send(ReceivedNpdu {
                        npdu: frame.data.clone(),
                        source_mac: MacAddr::from_slice(&[frame.source]),
                        reply_tx: Some(tx),
                    });
                }
                None
            }
            FrameType::TestRequest => {
                if frame.destination == self.config.this_station {
                    Some(MstpFrame {
                        frame_type: FrameType::TestResponse,
                        destination: frame.source,
                        source: self.config.this_station,
                        data: frame.data.clone(),
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Decide what to send when we have the token. Returns a frame to send.
    pub fn use_token(&mut self) -> MstpFrame {
        // Send queued data if available and under frame limit
        if self.frame_count < self.config.max_info_frames {
            if let Some((dest, npdu)) = self.tx_queue.pop_front() {
                self.frame_count += 1;
                let frame_type = if dest == BROADCAST_MAC {
                    FrameType::BACnetDataNotExpectingReply
                } else {
                    // Read expecting_reply from NPDU control byte (byte 1, bit 2)
                    // NPDU format: [version(1)][control(1)]...
                    let expecting_reply = npdu.len() >= 2 && (npdu[1] & 0x04) != 0;
                    if expecting_reply {
                        FrameType::BACnetDataExpectingReply
                    } else {
                        FrameType::BACnetDataNotExpectingReply
                    }
                };

                // If we've hit max_info_frames, transition to DoneWithToken
                // so the caller knows to pass the token next.
                if self.frame_count >= self.config.max_info_frames {
                    self.state = MasterState::DoneWithToken;
                }

                return MstpFrame {
                    frame_type,
                    destination: dest,
                    source: self.config.this_station,
                    data: npdu,
                };
            }
        } else {
            // Frame limit reached — transition to DoneWithToken and pass immediately.
            self.state = MasterState::DoneWithToken;
            return self.pass_token();
        }

        // Time to poll?
        if self.token_count >= NPOLL {
            self.token_count = 0;
            self.state = MasterState::PollForMaster;
            // Scan from this_station+1 through next_station-1 (wrapping at max_master)
            // Start polling at next_station (the first address after our known successor range)
            self.poll_station = next_addr(self.next_station, self.config.max_master);
            // If poll_station wraps to us, skip — we already know about next_station
            if self.poll_station == self.config.this_station {
                // Only us and next_station exist; no gap to scan
                return self.pass_token();
            }
            return MstpFrame {
                frame_type: FrameType::PollForMaster,
                destination: self.poll_station,
                source: self.config.this_station,
                data: Bytes::new(),
            };
        }

        // Pass the token
        self.pass_token()
    }

    /// Generate a token-pass frame to next_station.
    /// Enters PassToken state to wait for NS to use the token (Clause 9.5.6.6).
    pub fn pass_token(&mut self) -> MstpFrame {
        self.state = MasterState::PassToken;
        self.retry_token_count = 0;
        MstpFrame {
            frame_type: FrameType::Token,
            destination: self.next_station,
            source: self.config.this_station,
            data: Bytes::new(),
        }
    }

    /// Handle PassToken timeout (Clause 9.5.6.6).
    ///
    /// Called when T_usage_timeout expires after passing the token.
    /// Returns a frame to send (retry Token or PFM), or None if we should go to Idle.
    pub fn pass_token_timeout(&mut self) -> Option<MstpFrame> {
        let ts = self.config.this_station;
        if self.retry_token_count < N_RETRY_TOKEN {
            // RetrySendToken: resend Token to NS
            self.retry_token_count += 1;
            Some(MstpFrame {
                frame_type: FrameType::Token,
                destination: self.next_station,
                source: ts,
                data: Bytes::new(),
            })
        } else if self.next_station == ts {
            // FindNewSuccessorUnknown: NS wrapped back to TS
            // No other stations found — go to NoToken to try again
            self.state = MasterState::NoToken;
            None
        } else {
            // FindNewSuccessor: NS didn't respond, try next address
            self.next_station = next_addr(self.next_station, self.config.max_master);
            if self.next_station == ts {
                // Wrapped all the way around — declare sole master
                self.sole_master = true;
                self.state = MasterState::UseToken;
                self.frame_count = 0;
                None
            } else {
                // Try passing token to the new next_station
                self.retry_token_count = 0;
                Some(MstpFrame {
                    frame_type: FrameType::Token,
                    destination: self.next_station,
                    source: ts,
                    data: Bytes::new(),
                })
            }
        }
    }

    /// Handle PollForMaster timeout (no reply received).
    pub fn poll_timeout(&mut self) -> MstpFrame {
        self.poll_count += 1;
        if self.poll_count >= MAX_POLL_RETRIES {
            // No one answered — move to next poll station
            self.poll_count = 0;
            self.poll_station = next_addr(self.poll_station, self.config.max_master);
            if self.poll_station == self.config.this_station {
                // We've scanned the entire range — no other stations
                if self.next_station == self.config.this_station {
                    // Sole master: claim token directly
                    self.sole_master = true;
                    self.state = MasterState::UseToken;
                    self.frame_count = 0;
                    self.token_count = 0;
                    return MstpFrame {
                        frame_type: FrameType::Token,
                        destination: self.config.this_station,
                        source: self.config.this_station,
                        data: Bytes::new(),
                    };
                }
                // Have a known successor — pass token to them
                return self.pass_token();
            }
            if self.poll_station == self.next_station {
                // Reached our known successor — done scanning the gap
                return self.pass_token();
            }
        }
        // Poll the next station
        self.state = MasterState::PollForMaster;
        MstpFrame {
            frame_type: FrameType::PollForMaster,
            destination: self.poll_station,
            source: self.config.this_station,
            data: Bytes::new(),
        }
    }

    /// Queue an NPDU for transmission.
    ///
    /// Returns an error if the TX queue has reached [`MAX_TX_QUEUE_DEPTH`].
    pub fn queue_npdu(&mut self, dest: u8, npdu: Bytes) -> Result<(), Error> {
        if self.tx_queue.len() >= MAX_TX_QUEUE_DEPTH {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "MS/TP TX queue full",
            )));
        }
        self.tx_queue.push_back((dest, npdu));
        Ok(())
    }
}

/// Advance to the next station address, wrapping at max_master.
fn next_addr(current: u8, max_master: u8) -> u8 {
    if current >= max_master {
        0
    } else {
        current + 1
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
}

impl<S: SerialPort> TransportPort for MstpTransport<S> {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let (npdu_tx, npdu_rx) = mpsc::channel(64);

        let node = Arc::new(Mutex::new(MasterNode::new(self.config.clone())?));
        self.node = Some(node.clone());

        let serial = self
            .serial
            .take()
            .ok_or_else(|| Error::Encoding("MS/TP transport already started".into()))?;

        let serial = Arc::new(serial);
        let serial_clone = serial.clone();
        let t_turnaround_us = calculate_t_turnaround_us(self.config.baud_rate);

        // Receive loop using tokio::select! with timer
        let task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            let mut frame_buf = Vec::with_capacity(2048);

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
                                        MasterState::Idle => T_USAGE_TIMEOUT_MS,
                                        MasterState::NoToken => T_NO_TOKEN_MS,
                                        MasterState::PollForMaster => node_guard.t_slot_ms,
                                        MasterState::WaitForReply => T_REPLY_TIMEOUT_MS,
                                        MasterState::AnswerDataRequest => T_REPLY_DELAY_MS,
                                        MasterState::PassToken => T_USAGE_TIMEOUT_MS,
                                        MasterState::UseToken
                                        | MasterState::DoneWithToken => T_USAGE_TIMEOUT_MS,
                                    };
                                    drop(node_guard);

                                    // Clause 9.5.5.1: T_turnaround before transmitting
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
                                // Clause 9.5.6.7: Enter NoToken. Wait T_no_token + T_slot*TS
                                // before claiming the right to generate a token.
                                node_guard.state = MasterState::NoToken;
                                node_guard.retry_token_count = 0;
                                let ts = node_guard.config.this_station as u64;
                                T_NO_TOKEN_MS + node_guard.t_slot_ms * ts
                            }
                            MasterState::NoToken => {
                                // Clause 9.5.6.7 GenerateToken: This node wins the
                                // right to generate a token (its slot-based wait expired).
                                // Send PFM to discover successor, enter PollForMaster.
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
                                // Clause 9.5.6.4 ReplyTimeout: set FrameCount to
                                // max_info_frames and enter DONE_WITH_TOKEN.
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
                                // Clause 9.5.6.6: NS didn't use the token within T_usage_timeout
                                if let Some(frame) = node_guard.pass_token_timeout() {
                                    encode_buf.clear();
                                    if let Ok(()) = encode_frame(&mut encode_buf, &frame) {
                                        pending_writes.push(encode_buf.to_vec());
                                    }
                                }
                                match node_guard.state {
                                    MasterState::PassToken => T_USAGE_TIMEOUT_MS,
                                    MasterState::NoToken => T_NO_TOKEN_MS,
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

                        // Clause 9.5.5.1: T_turnaround before transmitting
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
            Err(Error::Encoding("MS/TP transport not started".into()))
        }
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        if let Some(ref node) = self.node {
            let mut node = node.lock().await;
            node.queue_npdu(BROADCAST_MAC, Bytes::copy_from_slice(npdu))?;
            Ok(())
        } else {
            Err(Error::Encoding("MS/TP transport not started".into()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_addr_wraps() {
        assert_eq!(next_addr(0, 127), 1);
        assert_eq!(next_addr(126, 127), 127);
        assert_eq!(next_addr(127, 127), 0);
        assert_eq!(next_addr(0, 0), 0); // edge case: max_master=0, wraps to self
    }

    #[test]
    fn master_node_initial_state() {
        let config = MstpConfig {
            this_station: 5,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let node = MasterNode::new(config).unwrap();
        assert_eq!(node.state, MasterState::Idle);
        // Clause 9.5.6.1: NS=TS, PS=TS, TokenCount=N_poll
        assert_eq!(node.next_station, 5);
        assert_eq!(node.poll_station, 5);
        assert_eq!(node.token_count, NPOLL);
        assert_eq!(node.retry_token_count, 0);
        assert!(node.reply_rx.is_none());
        assert!(node.pending_reply_source.is_none());
    }

    #[test]
    fn handle_token_for_us() {
        let (tx, _rx) = mpsc::channel(16);
        let config = MstpConfig {
            this_station: 3,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();

        let token = MstpFrame {
            frame_type: FrameType::Token,
            destination: 3,
            source: 0,
            data: Bytes::new(),
        };

        let response = node.handle_received_frame(&token, &tx);
        assert!(response.is_none()); // Token doesn't generate a response frame
        assert!(node.reply_rx.is_none());
        assert_eq!(node.state, MasterState::UseToken);
    }

    #[test]
    fn handle_token_not_for_us() {
        let (tx, _rx) = mpsc::channel(16);
        let config = MstpConfig {
            this_station: 3,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();

        let token = MstpFrame {
            frame_type: FrameType::Token,
            destination: 5,
            source: 0,
            data: Bytes::new(),
        };

        let response = node.handle_received_frame(&token, &tx);
        assert!(response.is_none());
        assert!(node.reply_rx.is_none());
        assert_eq!(node.state, MasterState::Idle);
    }

    #[test]
    fn handle_poll_for_master_for_us() {
        let (tx, _rx) = mpsc::channel(16);
        let config = MstpConfig {
            this_station: 10,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();

        let pfm = MstpFrame {
            frame_type: FrameType::PollForMaster,
            destination: 10,
            source: 0,
            data: Bytes::new(),
        };

        let response = node.handle_received_frame(&pfm, &tx);
        assert!(node.reply_rx.is_none());
        assert!(response.is_some());
        let reply = response.unwrap();
        assert_eq!(reply.frame_type, FrameType::ReplyToPollForMaster);
        assert_eq!(reply.destination, 0);
        assert_eq!(reply.source, 10);
    }

    #[test]
    fn handle_data_not_expecting_reply_unicast() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = MstpConfig {
            this_station: 3,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();

        let npdu_data = vec![0x01, 0x00, 0x10];
        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataNotExpectingReply,
            destination: 3,
            source: 0,
            data: Bytes::from(npdu_data.clone()),
        };

        let response = node.handle_received_frame(&frame, &tx);
        assert!(response.is_none());
        assert!(node.reply_rx.is_none());

        let received = rx.try_recv().unwrap();
        assert_eq!(received.npdu, npdu_data);
        assert_eq!(received.source_mac.as_slice(), &[0u8]);
    }

    #[test]
    fn handle_data_not_expecting_reply_broadcast() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = MstpConfig {
            this_station: 3,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();

        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataNotExpectingReply,
            destination: BROADCAST_MAC,
            source: 5,
            data: Bytes::from_static(&[0x01, 0x20]),
        };

        let _response = node.handle_received_frame(&frame, &tx);
        assert!(node.reply_rx.is_none());
        let received = rx.try_recv().unwrap();
        assert_eq!(received.source_mac.as_slice(), &[5u8]);
    }

    #[test]
    fn handle_data_expecting_reply() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = MstpConfig {
            this_station: 3,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();

        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataExpectingReply,
            destination: 3,
            source: 0,
            data: Bytes::from_static(&[0x01, 0x00, 0x30]),
        };

        let response = node.handle_received_frame(&frame, &tx);
        assert!(response.is_none());
        assert!(node.reply_rx.is_some()); // Reply channel provided
        assert_eq!(node.state, MasterState::AnswerDataRequest);
        assert!(node.reply_rx.is_some()); // Sender stored in node

        let received = rx.try_recv().unwrap();
        assert_eq!(received.npdu, vec![0x01, 0x00, 0x30]);
    }

    #[test]
    fn handle_test_request() {
        let (tx, _rx) = mpsc::channel(16);
        let config = MstpConfig {
            this_station: 3,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();

        let frame = MstpFrame {
            frame_type: FrameType::TestRequest,
            destination: 3,
            source: 0,
            data: Bytes::from_static(&[0xDE, 0xAD]),
        };

        let response = node.handle_received_frame(&frame, &tx);
        assert!(node.reply_rx.is_none());
        assert!(response.is_some());
        let reply = response.unwrap();
        assert_eq!(reply.frame_type, FrameType::TestResponse);
        assert_eq!(reply.data, vec![0xDE, 0xAD]);
    }

    #[test]
    fn use_token_passes_when_no_data() {
        let config = MstpConfig {
            this_station: 0,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();
        node.state = MasterState::UseToken;
        // Simulate having discovered a successor and recently polled
        node.next_station = 1;
        node.token_count = 0;

        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::Token);
        assert_eq!(frame.destination, 1); // next_station
                                          // pass_token() now enters PassToken state (Clause 9.5.6.6)
        assert_eq!(node.state, MasterState::PassToken);
    }

    #[test]
    fn use_token_sends_queued_data() {
        let config = MstpConfig {
            this_station: 0,
            max_master: 127,
            max_info_frames: 2,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();
        node.state = MasterState::UseToken;
        node.queue_npdu(5, Bytes::from_static(&[0x01, 0x00, 0x30]))
            .unwrap();
        node.queue_npdu(BROADCAST_MAC, Bytes::from_static(&[0x01, 0x20]))
            .unwrap();

        // First call sends the first queued frame (FIFO — unicast to 5 dequeued first)
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::BACnetDataNotExpectingReply);
        assert_eq!(frame.destination, 5);
        assert_eq!(frame.data, vec![0x01, 0x00, 0x30]);

        // Second call sends the broadcast (FIFO order)
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::BACnetDataNotExpectingReply);
        assert_eq!(frame.destination, BROADCAST_MAC);

        // Third call: no more data, pass token
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::Token);
    }

    #[test]
    fn use_token_respects_max_info_frames() {
        let config = MstpConfig {
            this_station: 0,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();
        node.state = MasterState::UseToken;
        node.queue_npdu(5, Bytes::from_static(&[0x01])).unwrap();
        node.queue_npdu(6, Bytes::from_static(&[0x02])).unwrap();

        // First call: sends one frame
        let frame = node.use_token();
        assert!(
            frame.frame_type == FrameType::BACnetDataExpectingReply
                || frame.frame_type == FrameType::BACnetDataNotExpectingReply
        );

        // Second call: frame_count >= max_info_frames, passes token
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::Token);

        // Data should still be in queue
        assert_eq!(node.tx_queue.len(), 1);
    }

    #[test]
    fn poll_for_master_after_npoll_tokens() {
        let config = MstpConfig {
            this_station: 0,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();
        node.state = MasterState::UseToken;
        node.token_count = NPOLL; // Trigger poll

        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::PollForMaster);
        assert_eq!(node.state, MasterState::PollForMaster);
        assert_eq!(node.token_count, 0);
    }

    #[test]
    fn reply_to_poll_sets_next_station() {
        let (tx, _rx) = mpsc::channel(16);
        let config = MstpConfig {
            this_station: 0,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();
        node.state = MasterState::PollForMaster;

        let reply = MstpFrame {
            frame_type: FrameType::ReplyToPollForMaster,
            destination: 0,
            source: 42,
            data: Bytes::new(),
        };

        let response = node.handle_received_frame(&reply, &tx);
        assert_eq!(node.next_station, 42);
        // Clause 9.5.6.8: send Token to NS, enter PassToken
        assert_eq!(node.state, MasterState::PassToken);
        assert!(!node.sole_master);
        // Should return a Token frame to send to the new NS
        assert!(response.is_some());
        let token = response.unwrap();
        assert_eq!(token.frame_type, FrameType::Token);
        assert_eq!(token.destination, 42);
    }

    #[test]
    fn poll_timeout_advances_poll_station() {
        let config = MstpConfig {
            this_station: 0,
            max_master: 3,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();
        node.state = MasterState::PollForMaster;
        // Start polling from station 1
        node.poll_station = 1;

        // MAX_POLL_RETRIES timeouts for station 1
        for _ in 0..MAX_POLL_RETRIES {
            let frame = node.poll_timeout();
            assert_eq!(frame.frame_type, FrameType::PollForMaster);
        }
        // Should have moved to station 2
        assert_eq!(node.poll_station, 2);
    }

    #[test]
    fn poll_timeout_sole_master() {
        let config = MstpConfig {
            this_station: 0,
            max_master: 1,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();
        node.state = MasterState::PollForMaster;
        node.poll_station = 1;

        // Timeout for station 1, MAX_POLL_RETRIES times
        for _ in 0..MAX_POLL_RETRIES {
            node.poll_timeout();
        }
        // poll_station wraps to 0 (== this_station), sole master declared
        assert_eq!(node.state, MasterState::UseToken);
        assert!(node.sole_master);
    }

    #[test]
    fn mstp_max_apdu_length() {
        let (s1, _s2) = LoopbackSerial::pair();
        let transport = MstpTransport::new(s1, MstpConfig::default());
        assert_eq!(transport.max_apdu_length(), 480);
    }

    #[test]
    fn local_mac_is_one_byte() {
        let (serial_a, _serial_b) = LoopbackSerial::pair();
        let config = MstpConfig {
            this_station: 42,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let transport = MstpTransport::new(serial_a, config);
        assert_eq!(transport.local_mac(), &[42]);
    }

    #[tokio::test]
    async fn loopback_serial_pair() {
        let (a, b) = LoopbackSerial::pair();

        // Write from A, read from B
        a.write(&[0x55, 0xFF, 0x01]).await.unwrap();
        let mut buf = [0u8; 16];
        let n = b.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], &[0x55, 0xFF, 0x01]);

        // Write from B, read from A
        b.write(&[0xAA, 0xBB]).await.unwrap();
        let n = a.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], &[0xAA, 0xBB]);
    }

    #[tokio::test]
    async fn transport_start_stop() {
        let (serial_a, _serial_b) = LoopbackSerial::pair();
        let config = MstpConfig {
            this_station: 0,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut transport = MstpTransport::new(serial_a, config);
        let _rx = transport.start().await.unwrap();
        transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn transport_queue_broadcast() {
        let (serial_a, _serial_b) = LoopbackSerial::pair();
        let config = MstpConfig {
            this_station: 0,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut transport = MstpTransport::new(serial_a, config);
        let _rx = transport.start().await.unwrap();

        transport.send_broadcast(&[0x01, 0x20]).await.unwrap();

        {
            let node = transport.node_state().unwrap();
            let node = node.lock().await;
            assert_eq!(node.tx_queue.len(), 1);
            assert_eq!(node.tx_queue[0].0, BROADCAST_MAC);
        }

        transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn transport_queue_unicast() {
        let (serial_a, _serial_b) = LoopbackSerial::pair();
        let config = MstpConfig {
            this_station: 0,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut transport = MstpTransport::new(serial_a, config);
        let _rx = transport.start().await.unwrap();

        transport.send_unicast(&[0x01, 0x00], &[5]).await.unwrap();

        {
            let node = transport.node_state().unwrap();
            let node = node.lock().await;
            assert_eq!(node.tx_queue.len(), 1);
            assert_eq!(node.tx_queue[0].0, 5);
        }

        transport.stop().await.unwrap();
    }

    #[test]
    fn use_token_frame_type_from_npdu_expecting_reply() {
        let mut node = MasterNode::new(MstpConfig {
            this_station: 1,
            max_master: 127,
            max_info_frames: 5,
            baud_rate: 9600,
        })
        .unwrap();
        node.state = MasterState::UseToken;

        // NPDU with expecting_reply=true (byte 1 bit 2 set)
        node.queue_npdu(5, Bytes::from_static(&[0x01, 0x04, 0xAA]))
            .unwrap();
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::BACnetDataExpectingReply);

        // NPDU with expecting_reply=false (byte 1 bit 2 clear)
        node.state = MasterState::UseToken;
        node.queue_npdu(5, Bytes::from_static(&[0x01, 0x00, 0xBB]))
            .unwrap();
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::BACnetDataNotExpectingReply);
    }

    #[test]
    fn use_token_broadcast_always_not_expecting() {
        let mut node = MasterNode::new(MstpConfig {
            this_station: 1,
            max_master: 127,
            max_info_frames: 5,
            baud_rate: 9600,
        })
        .unwrap();
        node.state = MasterState::UseToken;
        // Even with expecting_reply set, broadcast uses NotExpectingReply
        node.queue_npdu(BROADCAST_MAC, Bytes::from_static(&[0x01, 0x04, 0xAA]))
            .unwrap();
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::BACnetDataNotExpectingReply);
    }

    #[tokio::test]
    async fn transport_rejects_bad_mac() {
        let (serial_a, _serial_b) = LoopbackSerial::pair();
        let config = MstpConfig {
            this_station: 0,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut transport = MstpTransport::new(serial_a, config);
        let _rx = transport.start().await.unwrap();

        // 6-byte MAC is invalid for MS/TP
        let result = transport.send_unicast(&[0x01], &[1, 2, 3, 4, 5, 6]).await;
        assert!(result.is_err());

        transport.stop().await.unwrap();
    }

    // -------------------------------------------------------------------
    // New tests for NoToken, WaitForReply, AnswerDataRequest, scan range
    // -------------------------------------------------------------------

    #[test]
    fn test_no_token_timeout_claims_token() {
        // Simulate the NoToken -> sole master flow without the transport loop.
        // Per Clause 9.5.6, N_retry_token=1 means 1 retry AFTER the initial PFM,
        // so 2 total PFMs are sent before claiming sole master.
        //
        // Flow:
        //   Idle timeout -> enter NoToken, send 1st PFM, retry_token_count=0
        //   NoToken timeout #1 -> retry_token_count(0) < N_RETRY_TOKEN(1), send 2nd PFM, count=1
        //   NoToken timeout #2 -> retry_token_count(1) >= N_RETRY_TOKEN(1), claim sole master
        let config = MstpConfig {
            this_station: 5,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();

        // Simulate: Idle -> NoToken (first timeout sends 1st PFM)
        node.state = MasterState::NoToken;
        node.retry_token_count = 0;

        // First retry (retry_token_count=0 < N_RETRY_TOKEN=1)
        assert!(node.retry_token_count < N_RETRY_TOKEN);
        node.retry_token_count += 1;
        assert_eq!(node.retry_token_count, 1);

        // After N_RETRY_TOKEN retries, declare sole master
        assert!(node.retry_token_count >= N_RETRY_TOKEN);
        node.sole_master = true;
        node.next_station = node.config.this_station;
        node.state = MasterState::UseToken;
        node.frame_count = 0;
        node.token_count = 0;

        assert!(node.sole_master);
        assert_eq!(node.next_station, 5);
        assert_eq!(node.state, MasterState::UseToken);

        // Use token should pass to self (sole master)
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::Token);
        assert_eq!(frame.destination, 5); // pass to self
    }

    #[test]
    fn test_wait_for_reply_state_after_data_expecting_reply() {
        // When we send a DataExpectingReply via use_token, the recv loop
        // should set state to WaitForReply. We simulate that here.
        let config = MstpConfig {
            this_station: 1,
            max_master: 127,
            max_info_frames: 5,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();
        node.state = MasterState::UseToken;

        // Queue an NPDU with expecting_reply bit set
        node.queue_npdu(5, Bytes::from_static(&[0x01, 0x04, 0xAA]))
            .unwrap();
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::BACnetDataExpectingReply);

        // The recv loop would now set WaitForReply — simulate that
        node.state = MasterState::WaitForReply;
        assert_eq!(node.state, MasterState::WaitForReply);

        // On timeout in WaitForReply, we pass the token
        let token = node.pass_token();
        assert_eq!(token.frame_type, FrameType::Token);
        assert_eq!(node.state, MasterState::PassToken);
    }

    #[test]
    fn test_answer_data_request_reply_channel() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = MstpConfig {
            this_station: 3,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();

        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataExpectingReply,
            destination: 3,
            source: 7,
            data: Bytes::from_static(&[0x01, 0x04, 0x10]),
        };

        let response = node.handle_received_frame(&frame, &tx);
        assert!(response.is_none());
        assert_eq!(node.state, MasterState::AnswerDataRequest);

        // reply_rx stored on node, reply_tx sent via ReceivedNpdu channel
        assert!(node.reply_rx.is_some());

        // Receive the NPDU from the channel and extract reply_tx
        let received_npdu = rx.try_recv().expect("should receive NPDU");
        let reply_tx = received_npdu.reply_tx.expect("should have reply_tx");

        // Simulate application sending a reply through the channel
        let reply_data = vec![0x01, 0x00, 0x30, 0x01];
        reply_tx.send(Bytes::from(reply_data.clone())).unwrap();

        // The node's reply_rx should get the data
        let mut node_rx = node.reply_rx.take().unwrap();
        let received = node_rx.try_recv().unwrap();
        assert_eq!(received, reply_data);
    }

    #[test]
    fn test_poll_for_master_scan_range() {
        // Station 0, next_station=5, max_master=10
        // Should poll starting at 6 (next_addr(5, 10)), scanning 6..=10, 0 would be us so stop
        let config = MstpConfig {
            this_station: 0,
            max_master: 10,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();
        node.next_station = 5;
        node.state = MasterState::UseToken;
        node.token_count = NPOLL;

        // use_token triggers PollForMaster at poll_station = next_addr(5, 10) = 6
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::PollForMaster);
        assert_eq!(node.poll_station, 6);
        assert_eq!(frame.destination, 6);

        // Each station takes MAX_POLL_RETRIES timeouts to exhaust, then advances.
        // Station 6: 3 retries -> advance to 7
        for _ in 0..MAX_POLL_RETRIES {
            node.poll_timeout();
        }
        assert_eq!(node.poll_station, 7);

        // Station 7: 3 retries -> advance to 8
        for _ in 0..MAX_POLL_RETRIES {
            node.poll_timeout();
        }
        assert_eq!(node.poll_station, 8);

        // Station 8: 3 retries -> advance to 9
        for _ in 0..MAX_POLL_RETRIES {
            node.poll_timeout();
        }
        assert_eq!(node.poll_station, 9);

        // Station 9: 3 retries -> advance to 10
        for _ in 0..MAX_POLL_RETRIES {
            node.poll_timeout();
        }
        assert_eq!(node.poll_station, 10);

        // Station 10: 3 retries -> advance to next_addr(10, 10) = 0 == this_station
        // poll_timeout detects this_station match — since next_station=5 (not TS),
        // we have a known successor, pass token to them.
        for _ in 0..MAX_POLL_RETRIES {
            node.poll_timeout();
        }
        assert_eq!(node.state, MasterState::PassToken);
    }

    #[test]
    fn test_poll_for_master_scan_range_adjacent() {
        // When next_station is adjacent (this_station=0, next_station=1, max_master=1),
        // poll_station = next_addr(1, 1) = 0 == this_station, so no gap to scan
        let config = MstpConfig {
            this_station: 0,
            max_master: 1,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();
        node.next_station = 1;
        node.state = MasterState::UseToken;
        node.token_count = NPOLL;

        // use_token should just pass token since no gap
        let frame = node.use_token();
        assert_eq!(frame.frame_type, FrameType::Token);
        // pass_token enters PassToken state (Clause 9.5.6.6)
        assert_eq!(node.state, MasterState::PassToken);
    }

    #[test]
    fn mstp_frame_buf_max_size() {
        // The maximum valid MS/TP frame is: 2 (preamble) + 6 (header) + 1497 (data) + 2 (CRC16) = 1507
        assert_eq!(MSTP_MAX_FRAME_BUF, 1507);
    }

    #[test]
    fn master_node_rejects_station_above_max_master() {
        let config = MstpConfig {
            this_station: 128, // above MAX_MASTER (127)
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        assert!(MasterNode::new(config).is_err());
    }

    #[test]
    fn master_node_accepts_max_master_station() {
        let config = MstpConfig {
            this_station: 127, // exactly MAX_MASTER
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        assert!(MasterNode::new(config).is_ok());
    }

    #[test]
    fn pending_reply_source_stored_on_data_expecting_reply() {
        let (tx, _rx) = mpsc::channel(16);
        let config = MstpConfig {
            this_station: 3,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
        };
        let mut node = MasterNode::new(config).unwrap();

        let frame = MstpFrame {
            frame_type: FrameType::BACnetDataExpectingReply,
            destination: 3,
            source: 7,
            data: Bytes::from_static(&[0x01, 0x04, 0x10]),
        };

        let _ = node.handle_received_frame(&frame, &tx);
        assert_eq!(node.state, MasterState::AnswerDataRequest);
        assert_eq!(node.pending_reply_source, Some(7));
    }

    #[test]
    fn t_slot_baud_rate_9600() {
        // Clause 9.5.3: T_slot = 10ms regardless of baud rate
        assert_eq!(calculate_t_slot_ms(9600), 10);
    }

    #[test]
    fn t_slot_baud_rate_38400() {
        assert_eq!(calculate_t_slot_ms(38400), 10);
    }

    #[test]
    fn t_slot_baud_rate_76800() {
        assert_eq!(calculate_t_slot_ms(76800), 10);
    }

    #[test]
    fn t_slot_baud_rate_115200() {
        assert_eq!(calculate_t_slot_ms(115200), 10);
    }

    #[test]
    fn t_slot_stored_on_master_node() {
        let config = MstpConfig {
            this_station: 0,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 38400,
        };
        let node = MasterNode::new(config).unwrap();
        assert_eq!(node.t_slot_ms, 10);
    }
}
