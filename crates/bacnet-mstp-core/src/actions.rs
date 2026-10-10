//! Actions emitted by the MS/TP core and their bounded scheduler.
//!
//! The scheduler deliberately has two rings.  Protocol-control actions (turning
//! the transceiver around and putting bytes on the wire) have capacity that a
//! burst of received NPDUs cannot consume.  Payloads are copied into the rings
//! when they are queued; this is important because the stream decoder reuses
//! its receive buffer.

use bacnet_mstp_codec::{MAX_STANDARD_FRAME_LENGTH, MAX_STANDARD_MPDU_DATA};

/// The physical direction requested for the serial transceiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Receive bytes from the bus.
    Receive,
    /// Drive bytes onto the bus.
    Transmit,
}

/// A generation-bound identifier for an application request.
///
/// The core creates a fresh generation for each request.  Consumers should
/// return the identifier they received rather than manufacturing a new one;
/// the explicit constructor is provided so adapters can persist or restore
/// identifiers without depending on the representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(u32);

impl RequestId {
    /// Construct an identifier from a generation value.
    pub const fn new(generation: u32) -> Self {
        Self(generation)
    }

    /// Return the generation represented by this identifier.
    pub const fn generation(self) -> u32 {
        self.0
    }

    /// Return the raw generation value.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// A generation-bound identifier for a transmit operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransmitId(u32);

impl TransmitId {
    /// Construct an identifier from a generation value.
    pub const fn new(generation: u32) -> Self {
        Self(generation)
    }

    /// Return the generation represented by this identifier.
    pub const fn generation(self) -> u32 {
        self.0
    }

    /// Return the raw generation value.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Reason a platform could not complete a transmit operation.
///
/// The core does not know how a serial driver failed.  These deliberately
/// small, allocation-free reasons let the driver report the protocol-relevant
/// distinction without coupling the core to an operating-system error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransmitFailure {
    /// The driver stopped the transfer before the final stop bit was sent.
    Aborted,
    /// The transfer did not complete before its deadline.
    Timeout,
    /// The driver reported an I/O failure.
    Io,
    /// The driver rejected the transfer before it started.
    Rejected,
}

/// An operation emitted by the core.
///
/// The byte slices borrow the scheduler only for the duration of one poll.
/// A platform should copy a transmit frame into its UART/DMA storage before
/// polling the next action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action<'a> {
    /// Change the serial transceiver direction.
    SetDirection(Direction),
    /// Send one complete MS/TP frame.
    Transmit {
        /// Generation-bound operation identifier.
        id: TransmitId,
        /// Complete encoded frame, including preamble and CRCs.
        frame: &'a [u8],
    },
    /// Deliver a validated NPDU to the network layer.
    DeliverNpdu {
        /// Source MS/TP station.
        source: u8,
        /// Destination MS/TP station.
        destination: u8,
        /// NPDU bytes (without the MS/TP frame header and CRC).
        npdu: &'a [u8],
        /// Request awaiting an application decision, if this was a data-
        /// expecting-reply frame.
        request_id: Option<RequestId>,
    },
    /// No action is pending.  [`ActionRing::poll_action`] uses `None` for
    /// this condition so an idle poll does not need to borrow the ring.
    Idle,
}

/// Why an action could not be copied into a bounded ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionEnqueueError {
    /// The reserved protocol-control ring has no free slot.
    ControlFull,
    /// The application-delivery ring has no free slot.
    DeliveryFull,
    /// The encoded frame exceeds the codec's maximum frame size.
    FrameOversize { length: usize, maximum: usize },
    /// The NPDU exceeds the codec's standard maximum.
    NpduOversize { length: usize, maximum: usize },
}

const DEFAULT_CONTROL_CAPACITY: usize = 8;
const DEFAULT_DELIVERY_CAPACITY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlKind {
    Direction(Direction),
    Transmit {
        id: TransmitId,
        length: usize,
        frame: [u8; MAX_STANDARD_FRAME_LENGTH],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ControlSlot {
    sequence: u64,
    kind: ControlKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeliverySlot {
    sequence: u64,
    source: u8,
    destination: u8,
    request_id: Option<RequestId>,
    length: usize,
    npdu: [u8; MAX_STANDARD_MPDU_DATA],
}

/// Bounded action scheduler with independently reserved control and delivery
/// capacity.
///
/// `CONTROL` and `DELIVERY` are compile-time capacities.  The default alias
/// used by the core is [`ActionQueue`], while tests and small embedded ports
/// can select smaller rings.  All payload storage is inline and there is no
/// allocation after construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActionRing<const CONTROL: usize, const DELIVERY: usize> {
    controls: [Option<ControlSlot>; CONTROL],
    deliveries: [Option<DeliverySlot>; DELIVERY],
    next_sequence: u64,
    scratch_frame: [u8; MAX_STANDARD_FRAME_LENGTH],
    scratch_npdu: [u8; MAX_STANDARD_MPDU_DATA],
}

/// The standard scheduler capacity used by [`crate::MasterCore`].
pub(crate) type ActionQueue = ActionRing<DEFAULT_CONTROL_CAPACITY, DEFAULT_DELIVERY_CAPACITY>;

impl<const CONTROL: usize, const DELIVERY: usize> ActionRing<CONTROL, DELIVERY> {
    /// Construct an empty scheduler.
    pub(crate) const fn new() -> Self {
        Self {
            controls: [None; CONTROL],
            deliveries: [None; DELIVERY],
            next_sequence: 0,
            scratch_frame: [0; MAX_STANDARD_FRAME_LENGTH],
            scratch_npdu: [0; MAX_STANDARD_MPDU_DATA],
        }
    }

    /// Number of queued protocol-control actions.
    #[cfg(test)]
    pub(crate) fn control_len(&self) -> usize {
        self.controls.iter().filter(|slot| slot.is_some()).count()
    }

    /// Number of queued application-delivery actions.
    #[cfg(test)]
    pub(crate) fn delivery_len(&self) -> usize {
        self.deliveries.iter().filter(|slot| slot.is_some()).count()
    }

    /// Queue a transceiver direction change.
    pub(crate) fn enqueue_direction(
        &mut self,
        direction: Direction,
    ) -> Result<(), ActionEnqueueError> {
        let sequence = self.reserve_sequence();
        let slot = self
            .controls
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ActionEnqueueError::ControlFull)?;
        *slot = Some(ControlSlot {
            sequence,
            kind: ControlKind::Direction(direction),
        });
        Ok(())
    }

    /// Queue an encoded frame for transmission, copying its bytes into owned
    /// storage.
    #[cfg(test)]
    pub(crate) fn enqueue_transmit(
        &mut self,
        id: TransmitId,
        frame: &[u8],
    ) -> Result<(), ActionEnqueueError> {
        if frame.len() > MAX_STANDARD_FRAME_LENGTH {
            return Err(ActionEnqueueError::FrameOversize {
                length: frame.len(),
                maximum: MAX_STANDARD_FRAME_LENGTH,
            });
        }
        let sequence = self.reserve_sequence();
        let slot = self
            .controls
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ActionEnqueueError::ControlFull)?;
        let mut owned = [0; MAX_STANDARD_FRAME_LENGTH];
        owned[..frame.len()].copy_from_slice(frame);
        *slot = Some(ControlSlot {
            sequence,
            kind: ControlKind::Transmit {
                id,
                length: frame.len(),
                frame: owned,
            },
        });
        Ok(())
    }

    /// Queue the direction change and its frame as one atomic operation.
    ///
    /// A transmit must never be left in the queue without its preceding
    /// `Transmit` direction action.  The method therefore checks for two
    /// control slots before changing either ring entry; on failure it leaves
    /// the scheduler and its sequence counter untouched.
    pub(crate) fn enqueue_transmission(
        &mut self,
        id: TransmitId,
        frame: &[u8],
    ) -> Result<(), ActionEnqueueError> {
        if frame.len() > MAX_STANDARD_FRAME_LENGTH {
            return Err(ActionEnqueueError::FrameOversize {
                length: frame.len(),
                maximum: MAX_STANDARD_FRAME_LENGTH,
            });
        }
        let mut free = [0usize; 2];
        let mut count = 0;
        for (index, slot) in self.controls.iter().enumerate() {
            if slot.is_none() {
                free[count] = index;
                count += 1;
                if count == free.len() {
                    break;
                }
            }
        }
        if count != free.len() {
            return Err(ActionEnqueueError::ControlFull);
        }

        let direction_sequence = self.reserve_sequence();
        let transmit_sequence = self.reserve_sequence();
        let mut owned = [0; MAX_STANDARD_FRAME_LENGTH];
        owned[..frame.len()].copy_from_slice(frame);
        self.controls[free[0]] = Some(ControlSlot {
            sequence: direction_sequence,
            kind: ControlKind::Direction(Direction::Transmit),
        });
        self.controls[free[1]] = Some(ControlSlot {
            sequence: transmit_sequence,
            kind: ControlKind::Transmit {
                id,
                length: frame.len(),
                frame: owned,
            },
        });
        Ok(())
    }

    /// Queue an NPDU delivery, copying the bytes into owned storage.
    pub(crate) fn enqueue_delivery(
        &mut self,
        source: u8,
        destination: u8,
        npdu: &[u8],
        request_id: Option<RequestId>,
    ) -> Result<(), ActionEnqueueError> {
        if npdu.len() > MAX_STANDARD_MPDU_DATA {
            return Err(ActionEnqueueError::NpduOversize {
                length: npdu.len(),
                maximum: MAX_STANDARD_MPDU_DATA,
            });
        }
        let sequence = self.reserve_sequence();
        let slot = self
            .deliveries
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ActionEnqueueError::DeliveryFull)?;
        let mut owned = [0; MAX_STANDARD_MPDU_DATA];
        owned[..npdu.len()].copy_from_slice(npdu);
        *slot = Some(DeliverySlot {
            sequence,
            source,
            destination,
            request_id,
            length: npdu.len(),
            npdu: owned,
        });
        Ok(())
    }

    /// Remove the next action and borrow its payload from the scheduler's
    /// scratch storage.
    ///
    /// Removing the slot before constructing the returned action means a
    /// caller may enqueue more work after the returned borrow ends.  The
    /// scratch buffers are overwritten only by a subsequent poll, so no
    /// action contains a self-reference or an external borrowed payload.
    ///
    /// Protocol-control actions always preempt queued deliveries.  This is
    /// intentional: a full application ring must not delay restoring receive
    /// direction or sending the next token.
    pub(crate) fn poll_action(&mut self) -> Option<Action<'_>> {
        if let Some((control_index, _)) = self.oldest_control() {
            self.poll_control(control_index)
        } else {
            self.oldest_delivery()
                .and_then(|(delivery_index, _)| self.poll_delivery(delivery_index))
        }
    }

    fn reserve_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        sequence
    }

    fn oldest_control(&self) -> Option<(usize, u64)> {
        self.controls
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.map(|slot| (index, slot.sequence)))
            .min_by(|left, right| {
                if sequence_before(left.1, right.1) {
                    core::cmp::Ordering::Less
                } else if sequence_before(right.1, left.1) {
                    core::cmp::Ordering::Greater
                } else {
                    left.1.cmp(&right.1)
                }
            })
    }

    fn oldest_delivery(&self) -> Option<(usize, u64)> {
        self.deliveries
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.map(|slot| (index, slot.sequence)))
            .min_by(|left, right| {
                if sequence_before(left.1, right.1) {
                    core::cmp::Ordering::Less
                } else if sequence_before(right.1, left.1) {
                    core::cmp::Ordering::Greater
                } else {
                    left.1.cmp(&right.1)
                }
            })
    }

    fn poll_control(&mut self, index: usize) -> Option<Action<'_>> {
        let slot = self.controls[index].take()?;
        match slot.kind {
            ControlKind::Direction(direction) => Some(Action::SetDirection(direction)),
            ControlKind::Transmit { id, length, frame } => {
                self.scratch_frame[..length].copy_from_slice(&frame[..length]);
                Some(Action::Transmit {
                    id,
                    frame: &self.scratch_frame[..length],
                })
            }
        }
    }

    fn poll_delivery(&mut self, index: usize) -> Option<Action<'_>> {
        let slot = self.deliveries[index].take()?;
        self.scratch_npdu[..slot.length].copy_from_slice(&slot.npdu[..slot.length]);
        Some(Action::DeliverNpdu {
            source: slot.source,
            destination: slot.destination,
            npdu: &self.scratch_npdu[..slot.length],
            request_id: slot.request_id,
        })
    }
}

impl<const CONTROL: usize, const DELIVERY: usize> Default for ActionRing<CONTROL, DELIVERY> {
    fn default() -> Self {
        Self::new()
    }
}

/// Compare sequence numbers using serial-number arithmetic.
///
/// A ring can never retain enough entries for the difference to approach
/// half of the `u64` sequence space, so this remains unambiguous across a
/// wrap.
fn sequence_before(left: u64, right: u64) -> bool {
    (left.wrapping_sub(right) as i64) < 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_preempt_deliveries_and_delivery_order_is_stable() {
        let mut actions = ActionRing::<2, 2>::new();
        actions.enqueue_delivery(1, 2, &[1], None).unwrap();
        actions.enqueue_direction(Direction::Transmit).unwrap();
        actions
            .enqueue_transmit(TransmitId::new(9), &[0x55, 0xFF])
            .unwrap();
        actions
            .enqueue_delivery(3, 4, &[2], Some(RequestId::new(7)))
            .unwrap();

        assert_eq!(
            actions.poll_action(),
            Some(Action::SetDirection(Direction::Transmit))
        );
        assert_eq!(
            actions.poll_action(),
            Some(Action::Transmit {
                id: TransmitId::new(9),
                frame: &[0x55, 0xFF],
            })
        );
        assert_eq!(
            actions.poll_action(),
            Some(Action::DeliverNpdu {
                source: 1,
                destination: 2,
                npdu: &[1],
                request_id: None,
            })
        );
        assert_eq!(
            actions.poll_action(),
            Some(Action::DeliverNpdu {
                source: 3,
                destination: 4,
                npdu: &[2],
                request_id: Some(RequestId::new(7)),
            })
        );
        assert_eq!(actions.poll_action(), None);
    }

    #[test]
    fn delivery_saturation_does_not_consume_control_capacity() {
        let mut actions = ActionRing::<2, 1>::new();
        actions.enqueue_delivery(1, 2, &[0xAA], None).unwrap();
        assert_eq!(
            actions.enqueue_delivery(1, 2, &[0xBB], None),
            Err(ActionEnqueueError::DeliveryFull)
        );
        actions.enqueue_direction(Direction::Transmit).unwrap();
        actions.enqueue_direction(Direction::Receive).unwrap();
        assert_eq!(actions.control_len(), 2);
        assert_eq!(actions.delivery_len(), 1);
    }

    #[test]
    fn transmission_reserves_direction_atomically() {
        let mut actions = ActionRing::<1, 1>::new();
        assert_eq!(
            actions.enqueue_transmission(TransmitId::new(4), &[0x55]),
            Err(ActionEnqueueError::ControlFull)
        );
        assert_eq!(actions.control_len(), 0);
        actions.enqueue_direction(Direction::Receive).unwrap();

        let mut actions = ActionRing::<2, 1>::new();
        actions
            .enqueue_transmission(TransmitId::new(4), &[0x55])
            .unwrap();
        assert_eq!(actions.control_len(), 2);
        assert_eq!(
            actions.poll_action(),
            Some(Action::SetDirection(Direction::Transmit))
        );
        assert_eq!(
            actions.poll_action(),
            Some(Action::Transmit {
                id: TransmitId::new(4),
                frame: &[0x55],
            })
        );
    }

    #[test]
    fn queued_payload_is_stable_after_source_changes() {
        let mut actions = ActionRing::<1, 1>::new();
        let mut frame = [1, 2, 3];
        actions
            .enqueue_transmit(TransmitId::new(1), &frame)
            .unwrap();
        frame.fill(0xFF);
        let action = actions.poll_action().unwrap();
        assert_eq!(
            action,
            Action::Transmit {
                id: TransmitId::new(1),
                frame: &[1, 2, 3],
            }
        );

        let mut npdu = [4, 5, 6];
        actions.enqueue_delivery(4, 5, &npdu, None).unwrap();
        npdu.fill(0xEE);
        assert_eq!(
            actions.poll_action(),
            Some(Action::DeliverNpdu {
                source: 4,
                destination: 5,
                npdu: &[4, 5, 6],
                request_id: None,
            })
        );
    }

    #[test]
    fn rejects_oversize_payloads_without_consuming_slots() {
        let mut actions = ActionRing::<1, 1>::new();
        let frame = [0u8; MAX_STANDARD_FRAME_LENGTH + 1];
        let npdu = [0u8; MAX_STANDARD_MPDU_DATA + 1];
        assert_eq!(
            actions.enqueue_transmit(TransmitId::new(1), &frame),
            Err(ActionEnqueueError::FrameOversize {
                length: MAX_STANDARD_FRAME_LENGTH + 1,
                maximum: MAX_STANDARD_FRAME_LENGTH,
            })
        );
        assert_eq!(
            actions.enqueue_delivery(0, 1, &npdu, None),
            Err(ActionEnqueueError::NpduOversize {
                length: MAX_STANDARD_MPDU_DATA + 1,
                maximum: MAX_STANDARD_MPDU_DATA,
            })
        );
        actions.enqueue_direction(Direction::Receive).unwrap();
        assert_eq!(actions.control_len(), 1);
        assert_eq!(actions.delivery_len(), 0);
    }

    #[test]
    fn sequence_comparison_handles_wrap() {
        let mut actions = ActionRing::<2, 1>::new();
        actions.next_sequence = u64::MAX;
        actions.enqueue_direction(Direction::Transmit).unwrap();
        actions.enqueue_direction(Direction::Receive).unwrap();
        assert_eq!(
            actions.poll_action(),
            Some(Action::SetDirection(Direction::Transmit))
        );
        assert_eq!(
            actions.poll_action(),
            Some(Action::SetDirection(Direction::Receive))
        );
    }
}
