//! This node's own management exchanges: a request to a peer and the one
//! response that settles it.
//!
//! A BVLC-Result carries no invoke ID, so a response is matched by who sent
//! it and what it can answer. At most one exchange is live per peer and
//! operation, and at most one per peer among the operations a success can
//! answer, so every response has at most one exchange it fits. Each
//! admitted exchange takes the next local generation, which its
//! [`ExchangeId`] carries: a handle kept past its exchange's end never
//! touches a newer one in the same slot.

use core::fmt;
use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BvlcFunction, BvlcResultCode};
use bacnet_time::{Duration, Instant};

/// A management request this node can make, and expect an answer to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Read-Broadcast-Distribution-Table: a Read-BDT-Ack or its NAK.
    ReadBdt,
    /// Write-Broadcast-Distribution-Table: success or its NAK.
    WriteBdt,
    /// Read-Foreign-Device-Table: a Read-FDT-Ack or its NAK.
    ReadFdt,
    /// Delete-Foreign-Device-Table-Entry: success or its NAK.
    DeleteFdtEntry,
    /// Register-Foreign-Device: success or its NAK.
    RegisterForeignDevice,
}

impl Operation {
    /// The five operations.
    pub const ALL: [Self; 5] = [
        Self::ReadBdt,
        Self::WriteBdt,
        Self::ReadFdt,
        Self::DeleteFdtEntry,
        Self::RegisterForeignDevice,
    ];

    /// The operation a request of `function` is, if any.
    #[must_use]
    pub const fn of(function: BvlcFunction) -> Option<Self> {
        Some(match function {
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE => Self::ReadBdt,
            BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE => Self::WriteBdt,
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE => Self::ReadFdt,
            BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => Self::DeleteFdtEntry,
            BvlcFunction::REGISTER_FOREIGN_DEVICE => Self::RegisterForeignDevice,
            _ => return None,
        })
    }

    /// The request's function.
    #[must_use]
    pub const fn function(self) -> BvlcFunction {
        match self {
            Self::ReadBdt => BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            Self::WriteBdt => BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
            Self::ReadFdt => BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
            Self::DeleteFdtEntry => BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
            Self::RegisterForeignDevice => BvlcFunction::REGISTER_FOREIGN_DEVICE,
        }
    }

    /// The operation whose NAK is `code`, if any of the five: a
    /// Distribute-Broadcast-To-Network NAK names none.
    #[must_use]
    pub const fn of_nak(code: BvlcResultCode) -> Option<Self> {
        let mut index = 0;
        while index < Self::ALL.len() {
            let operation = Self::ALL[index];
            if let Some(nak) = operation.function().nak() {
                if nak.to_raw() == code.to_raw() {
                    return Some(operation);
                }
            }
            index += 1;
        }
        None
    }

    /// The ACK that answers it, for the two table reads.
    #[must_use]
    pub const fn ack(self) -> Option<BvlcFunction> {
        match self {
            Self::ReadBdt => Some(BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK),
            Self::ReadFdt => Some(BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK),
            _ => None,
        }
    }

    /// The operation a table ACK of `function` answers.
    #[must_use]
    pub const fn of_ack(function: BvlcFunction) -> Option<Self> {
        match function {
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK => Some(Self::ReadBdt),
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK => Some(Self::ReadFdt),
            _ => None,
        }
    }

    /// Whether a success answers it. A success names no operation, so a
    /// peer has at most one of these live at a time.
    #[must_use]
    pub const fn answered_by_success(self) -> bool {
        matches!(
            self,
            Self::WriteBdt | Self::DeleteFdtEntry | Self::RegisterForeignDevice
        )
    }
}

/// One admitted exchange: its slot and its generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExchangeId {
    slot: u8,
    generation: u32,
}

impl ExchangeId {
    pub(crate) const fn new(slot: usize, generation: u32) -> Self {
        Self {
            slot: slot as u8,
            generation,
        }
    }

    /// The slot it occupies.
    #[must_use]
    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    /// Its generation, unique among the node's exchanges until the counter
    /// wraps.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

/// Why an exchange wasn't admitted. Nothing was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExchangeRefusal {
    /// The same operation to the same peer is live.
    SameExchange,
    /// Another operation a success answers is live to the same peer, and a
    /// success couldn't tell the two apart.
    ResultClass,
    /// Every slot is in use.
    Full,
    /// The message isn't one of the five requests ([`Operation`]).
    NotARequest,
    /// A foreign device's own BBMD, for a request a success answers
    /// (Register-Foreign-Device, Delete-Foreign-Device-Table-Entry,
    /// Write-Broadcast-Distribution-Table). The node owns that
    /// registration: such a request could change the BBMD's entry behind
    /// the lease, and its success could pass for a registration's.
    ManagedRegistration,
}

impl fmt::Display for ExchangeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SameExchange => "the same BVLC request to this peer is in flight",
            Self::ResultClass => {
                "another BVLC request answered by success is in flight to this peer"
            }
            Self::Full => "every BVLC exchange slot is in use",
            Self::NotARequest => "not a BVLC management request",
            Self::ManagedRegistration => {
                "the foreign-device registration with this BBMD is managed automatically"
            }
        })
    }
}

impl core::error::Error for ExchangeRefusal {}

/// Why the caller ended an exchange itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LocalFailure {
    /// Encoding or sending the request failed.
    SendFailed,
    /// The caller gave up waiting.
    Cancelled,
}

/// What a response that beat the request's send said, kept until the send
/// is confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Early {
    /// A BVLC-Result's code; `None` for a table ACK.
    pub(crate) code: Option<BvlcResultCode>,
    /// Whether it was accepted (decoded, and taken by the policy).
    pub(crate) valid: bool,
}

/// Where an exchange is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// No exchange.
    Free,
    /// Admitted; the send isn't confirmed yet.
    Admitted,
    /// Sent; it times out at the deadline.
    Sent { deadline: Instant },
}

/// One slot: an exchange, or the last one's key while its late results
/// may still arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Slot {
    pub(crate) phase: Phase,
    pub(crate) peer: SocketAddrV4,
    pub(crate) operation: Operation,
    pub(crate) generation: u32,
    pub(crate) timeout: Duration,
    /// An automatic registration attempt, which the lifecycle owns.
    pub(crate) automatic: bool,
    /// The time-to-live a Register-Foreign-Device asked for.
    pub(crate) ttl: u16,
    pub(crate) early: Option<Early>,
    /// Once a sent exchange ends: until when a result for its key counts
    /// as late.
    pub(crate) late_until: Option<Instant>,
}

impl Slot {
    const FREE: Self = Self {
        phase: Phase::Free,
        peer: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
        operation: Operation::ReadBdt,
        generation: 0,
        timeout: Duration::ZERO,
        automatic: false,
        ttl: 0,
        early: None,
        late_until: None,
    };

    fn is_live(&self) -> bool {
        self.phase != Phase::Free
    }

    fn id(&self, slot: usize) -> ExchangeId {
        ExchangeId::new(slot, self.generation)
    }
}

/// What a well-formed response can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    /// A success: whichever operation a success answers.
    Success,
    /// A NAK or an ACK: this operation.
    Operation(Operation),
}

impl Key {
    /// The key of a response of `function` carrying `code` (a
    /// BVLC-Result's), or `None` when it answers nothing this node asks,
    /// such as an unknown code or a Distribute-Broadcast-To-Network NAK.
    pub(crate) fn of(function: BvlcFunction, code: Option<BvlcResultCode>) -> Option<Self> {
        match code {
            Some(BvlcResultCode::SUCCESSFUL_COMPLETION) => Some(Self::Success),
            Some(code) => Operation::of_nak(code).map(Self::Operation),
            None => Operation::of_ack(function).map(Self::Operation),
        }
    }

    fn fits(self, operation: Operation) -> bool {
        match self {
            Self::Success => operation.answered_by_success(),
            Self::Operation(own) => own == operation,
        }
    }
}

/// `X` exchange slots and the generation counter.
#[derive(Debug, Clone)]
pub(crate) struct Exchanges<const X: usize> {
    slots: [Slot; X],
    generation: u32,
}

impl<const X: usize> Exchanges<X> {
    pub(crate) const fn new() -> Self {
        Self {
            slots: [Slot::FREE; X],
            generation: 0,
        }
    }

    /// Admit an exchange of `operation` with `peer`, or say why not.
    pub(crate) fn admit(
        &mut self,
        peer: SocketAddrV4,
        operation: Operation,
        timeout: Duration,
        automatic: bool,
        ttl: u16,
    ) -> Result<ExchangeId, ExchangeRefusal> {
        for slot in self
            .slots
            .iter()
            .filter(|slot| slot.is_live() && slot.peer == peer)
        {
            if slot.operation == operation {
                return Err(ExchangeRefusal::SameExchange);
            }
            if operation.answered_by_success() && slot.operation.answered_by_success() {
                return Err(ExchangeRefusal::ResultClass);
            }
        }
        let index = self
            .slots
            .iter()
            .position(|slot| !slot.is_live())
            .ok_or(ExchangeRefusal::Full)?;
        self.generation = self.generation.wrapping_add(1);
        self.slots[index] = Slot {
            phase: Phase::Admitted,
            peer,
            operation,
            generation: self.generation,
            timeout,
            automatic,
            ttl,
            early: None,
            late_until: None,
        };
        Ok(self.slots[index].id(index))
    }

    /// The slot `id` names, while its exchange is live.
    pub(crate) fn live(&self, id: ExchangeId) -> Option<usize> {
        let slot = self.slots.get(id.slot())?;
        (slot.is_live() && slot.generation == id.generation).then_some(id.slot())
    }

    pub(crate) fn slot(&self, index: usize) -> &Slot {
        &self.slots[index]
    }

    pub(crate) fn slot_mut(&mut self, index: usize) -> &mut Slot {
        &mut self.slots[index]
    }

    pub(crate) fn id(&self, index: usize) -> ExchangeId {
        self.slots[index].id(index)
    }

    /// The live exchange with `peer` that a response with `key` answers.
    pub(crate) fn find(&self, peer: SocketAddrV4, key: Key) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.is_live() && slot.peer == peer && key.fits(slot.operation))
    }

    /// The first live exchange with `peer`, in slot order, that a response
    /// of `function` could be for: any, for a BVLC-Result; the table read,
    /// for an ACK. A malformed response is counted against it.
    pub(crate) fn first_with(&self, peer: SocketAddrV4, function: BvlcFunction) -> Option<usize> {
        let read = Operation::of_ack(function);
        self.slots.iter().position(|slot| {
            slot.is_live()
                && slot.peer == peer
                && (function == BvlcFunction::BVLC_RESULT || Some(slot.operation) == read)
        })
    }

    /// Whether a response with `key` from `peer` comes within one response
    /// timeout of a sent exchange it fits ending.
    pub(crate) fn is_late(&self, peer: SocketAddrV4, key: Key, now: Instant) -> bool {
        self.slots.iter().any(|slot| {
            !slot.is_live()
                && slot.peer == peer
                && key.fits(slot.operation)
                && slot.late_until.is_some_and(|until| now < until)
        })
    }

    /// End the exchange in `index`, and return what it was. A sent one
    /// leaves its key for one response timeout, to tell late results apart.
    pub(crate) fn retire(&mut self, index: usize, now: Instant) -> Slot {
        let ended = self.slots[index];
        let slot = &mut self.slots[index];
        slot.late_until = matches!(ended.phase, Phase::Sent { .. }).then(|| now + ended.timeout);
        slot.phase = Phase::Free;
        slot.early = None;
        ended
    }

    /// The live automatic registration attempt, if one is in flight.
    pub(crate) fn automatic(&self) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.is_live() && slot.automatic)
    }

    /// The sent exchange whose deadline comes first, and that deadline.
    pub(crate) fn next_deadline(&self) -> Option<(usize, Instant)> {
        let mut first: Option<(usize, Instant)> = None;
        for (index, slot) in self.slots.iter().enumerate() {
            if let Phase::Sent { deadline } = slot.phase {
                if first.is_none_or(|(_, earliest)| deadline < earliest) {
                    first = Some((index, deadline));
                }
            }
        }
        first
    }
}
