//! A B/IP datagram as its BVLL header and the NPDU it carries, so a send
//! needs no buffer of its own (#1978 item 5).
//!
//! Encoding a message copies the NPDU behind its header into one buffer, an
//! allocation and a copy for every NPDU sent. An [`OutgoingDatagram`] keeps
//! the header in a few octets of its own and borrows the NPDU, and the socket
//! sends the two as one datagram with a vectored send.

use std::io;

use bacnet_bip_codec::{Bvll, BvllSink, SinkFull};

/// The most header octets a datagram keeps: a Forwarded-NPDU's ten, the
/// longest header in front of an NPDU.
const HEAD_ROOM: usize = 10;

/// One datagram: `head`, then `payload`. A whole frame encoded elsewhere is a
/// datagram with no head.
#[derive(Clone, Copy)]
pub(super) struct OutgoingDatagram<'a> {
    head: [u8; HEAD_ROOM],
    head_len: u8,
    payload: &'a [u8],
}

impl<'a> OutgoingDatagram<'a> {
    /// A frame already encoded in one buffer.
    pub(super) fn whole(frame: &'a [u8]) -> Self {
        Self {
            head: [0; HEAD_ROOM],
            head_len: 0,
            payload: frame,
        }
    }

    /// `message` as its header and `npdu`, the NPDU it carries, when the
    /// encoder writes `npdu` last and in one piece. `None` otherwise, which
    /// the unicast send reports as an encoding error: an
    /// Original-Unicast-NPDU always splits.
    pub(super) fn split(message: &Bvll<'_>, npdu: &'a [u8]) -> Option<Self> {
        let head_len = message.encoded_len().ok()?.checked_sub(npdu.len())?;
        if head_len > HEAD_ROOM {
            return None;
        }
        let mut sink = HeadSink {
            head: [0; HEAD_ROOM],
            head_len,
            written: 0,
            npdu,
            npdu_seen: false,
            mismatch: false,
        };
        message.encode_to_sink(&mut sink).ok()?;
        if sink.mismatch || !sink.npdu_seen || sink.written != head_len {
            return None;
        }
        Some(Self {
            head: sink.head,
            head_len: head_len as u8,
            payload: npdu,
        })
    }

    /// The header, empty for a whole frame.
    pub(super) fn head(&self) -> &[u8] {
        &self.head[..usize::from(self.head_len)]
    }

    /// The octets after the header.
    pub(super) fn payload(&self) -> &'a [u8] {
        self.payload
    }

    /// The datagram's length.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        usize::from(self.head_len) + self.payload.len()
    }

    /// The datagram in one buffer, for the tap and tests.
    pub(super) fn joined(&self) -> Vec<u8> {
        [self.head(), self.payload].concat()
    }

    /// Send it to `target` once, without waiting for the socket.
    pub(super) fn try_send(
        &self,
        socket: &tokio::net::UdpSocket,
        target: std::net::SocketAddrV4,
    ) -> io::Result<usize> {
        if self.head_len == 0 {
            return socket.try_send_to(self.payload, target.into());
        }
        let parts = [
            io::IoSlice::new(self.head()),
            io::IoSlice::new(self.payload),
        ];
        let address = socket2::SockAddr::from(target);
        socket.try_io(tokio::io::Interest::WRITABLE, || {
            socket2::SockRef::from(socket).send_to_vectored(&parts, &address)
        })
    }
}

/// Keeps a message's header and checks that the rest is `npdu`, as one write.
struct HeadSink<'n> {
    head: [u8; HEAD_ROOM],
    head_len: usize,
    written: usize,
    npdu: &'n [u8],
    npdu_seen: bool,
    mismatch: bool,
}

impl BvllSink for HeadSink<'_> {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        if self.written < self.head_len {
            let end = self.written + bytes.len();
            match self.head.get_mut(self.written..end) {
                Some(room) if end <= self.head_len => room.copy_from_slice(bytes),
                _ => self.mismatch = true,
            }
            self.written = end;
        } else if !self.npdu_seen
            && bytes.as_ptr() == self.npdu.as_ptr()
            && bytes.len() == self.npdu.len()
        {
            self.npdu_seen = true;
        } else {
            self.mismatch = true;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "datagram_tests.rs"]
mod tests;
