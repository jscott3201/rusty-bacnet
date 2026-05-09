//! MS/TP (Master-Slave/Token-Passing) transport per ASHRAE 135-2020 Clause 9.
//!
//! Implements the master node state machine for token-passing over RS-485.
//! The actual serial I/O is abstracted behind the [`SerialPort`] trait so that
//! the state machine can be tested without hardware.

use std::collections::VecDeque;

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tracing::debug;

use crate::mstp_frame::{FrameType, MstpFrame, BROADCAST_MAC, MAX_MASTER};
use crate::port::ReceivedNpdu;

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
// MS/TP timing constants
// ---------------------------------------------------------------------------

/// Time without a token before a node assumes the token is lost (ms).
const T_NO_TOKEN_MS: u64 = 500;
/// Time to wait for a reply after sending DataExpectingReply (ms).
const T_REPLY_TIMEOUT_MS: u64 = 255;
/// Time to wait for another node to begin using the token after it was passed (ms).
const T_USAGE_TIMEOUT_MS: u64 = 20;
/// The width of the time slot within which a node may generate a token (ms).
fn calculate_t_slot_ms(_baud_rate: u32) -> u64 {
    10
}
/// Maximum time a node may delay before sending a reply to DataExpectingReply (ms).
const T_REPLY_DELAY_MS: u64 = 250;
/// Minimum silence time (40 bit times) before transmitting after receiving last octet.
fn calculate_t_turnaround_us(baud_rate: u32) -> u64 {
    40_000_000u64 / baud_rate as u64
}
/// Number of retries for token pass before declaring token lost.
const N_RETRY_TOKEN: u8 = 1;
/// Maximum frame buffer size: preamble(2) + header(6) + max data(1497) + CRC16(2)
pub(crate) const MSTP_MAX_FRAME_BUF: usize = 1507;
/// Maximum inter-byte gap within a frame before aborting reception.
/// Spec Clause 9.5.5: minimum 60 bit times. Computed per baud rate.
fn calculate_t_frame_abort_us(baud_rate: u32) -> u64 {
    // 60 bit times in microseconds, rounded up
    60_000_000u64.div_ceil(baud_rate as u64)
}
/// Maximum number of queued outgoing frames before rejecting new sends.
const MAX_TX_QUEUE_DEPTH: usize = 256;

// ---------------------------------------------------------------------------
// Master node state machine
// ---------------------------------------------------------------------------

/// Token-passing master node state.
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
    /// Token has been passed to NS, waiting for NS to use it.
    PassToken,
    /// Waiting for a reply after sending DataExpectingReply.
    WaitForReply,
    /// Polling for successor master stations.
    PollForMaster,
    /// Answering a received DataExpectingReply.
    AnswerDataRequest,
}

/// Master node configuration.
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
    /// Number of valid octets/frames received (used in PassToken and NoToken states).
    pub event_count: u32,
    /// Station address we sent a DataExpectingReply to (for WAIT_FOR_REPLY validation).
    pub expected_reply_source: Option<u8>,
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
            expected_reply_source: None,
        })
    }

    /// Handle a received frame. Returns an optional response frame to send on the wire,
    /// and an optional oneshot receiver for reply data (only when entering AnswerDataRequest).
    pub fn handle_received_frame(
        &mut self,
        frame: &MstpFrame,
        npdu_tx: &mpsc::Sender<ReceivedNpdu>,
    ) -> Option<MstpFrame> {
        self.event_count = self.event_count.saturating_add(1);

        // SawTokenUser: NS has started using the token.
        if self.state == MasterState::PassToken {
            self.state = MasterState::Idle;
        }
        match frame.frame_type {
            FrameType::Token => {
                if frame.destination == self.config.this_station {
                    debug!(src = frame.source, "received token");
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
                if self.state == MasterState::WaitForReply
                    && frame.destination == self.config.this_station
                {
                    // ReceivedReply or ReceivedUnexpectedFrame — validate source
                    if self.expected_reply_source == Some(frame.source) {
                        let _ = npdu_tx.try_send(ReceivedNpdu {
                            npdu: frame.data.clone(),
                            source_mac: MacAddr::from_slice(&[frame.source]),
                            reply_tx: None,
                        });
                    }
                    // Both cases → DoneWithToken per spec Clause 9.5.6
                    self.expected_reply_source = None;
                    self.state = MasterState::DoneWithToken;
                } else if frame.destination == self.config.this_station
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
            FrameType::ReplyPostponed => {
                if self.state == MasterState::WaitForReply
                    && frame.destination == self.config.this_station
                {
                    // Reply will come later when the replying node gets the token
                    self.expected_reply_source = None;
                    self.state = MasterState::DoneWithToken;
                }
                None
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

                // Track expected reply source for WAIT_FOR_REPLY validation
                if frame_type == FrameType::BACnetDataExpectingReply {
                    self.expected_reply_source = Some(dest);
                }

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

    /// Handle PassToken timeout.
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

mod port;
pub use port::{LoopbackSerial, MstpTransport, NoSerial};

#[cfg(test)]
mod tests;
