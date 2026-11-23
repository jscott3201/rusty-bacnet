//! The zone a peer wrote that the responder's clock hasn't taken yet
//! (#2395), shared by that clock and every copy it lends the database.
//!
//! The Device writes through the copy it was lent, which can't reach the
//! responder's own clock; it leaves the zone here instead, and every copy
//! reads in it until the responder takes it. One word holds it, so the
//! cell needs no lock and no allocation beyond the clock's `Arc`: the
//! offset's 16 bits, the daylight-saving bit above them, and a bit saying
//! a zone is held. The host runs a write with the database held
//! exclusively, so nothing reads the cell while a write changes it.

use core::sync::atomic::{AtomicU32, Ordering};

use super::Zone;

/// Set while a written zone waits.
const HELD: u32 = 1 << 31;
/// The daylight-saving bit.
const SUMMER: u32 = 1 << 16;

#[derive(Debug, Default)]
pub(super) struct ZoneWrites(AtomicU32);

impl ZoneWrites {
    /// Hold `zone`, in place of any zone held before.
    pub(super) fn put(&self, zone: Zone) {
        let offset = u32::from(zone.utc_offset() as u16);
        let summer = if zone.daylight_savings() { SUMMER } else { 0 };
        self.0.store(HELD | summer | offset, Ordering::Release);
    }

    /// The zone held, if any.
    pub(super) fn peek(&self) -> Option<Zone> {
        decode(self.0.load(Ordering::Acquire))
    }

    /// The zone held, if any, holding none from now on. A load and a store
    /// rather than a swap: the host holds the database exclusively, so no
    /// write comes between them, and a target without atomic swaps builds
    /// it.
    pub(super) fn take(&self) -> Option<Zone> {
        let zone = self.peek()?;
        self.0.store(0, Ordering::Release);
        Some(zone)
    }
}

/// The zone a held word names; every word [`ZoneWrites::put`] stores is one.
fn decode(word: u32) -> Option<Zone> {
    if word & HELD == 0 {
        return None;
    }
    let offset = (word & 0xFFFF) as u16 as i16;
    Zone::new(offset, word & SUMMER != 0).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_zone_comes_back_once_whatever_its_sign() {
        let writes = ZoneWrites::default();
        assert_eq!(writes.peek(), None);
        for (offset, summer) in [(0, false), (-1440, true), (1440, false), (-15, true)] {
            let zone = Zone::new(offset, summer).unwrap();
            writes.put(zone);
            assert_eq!(writes.peek(), Some(zone));
            assert_eq!(writes.take(), Some(zone));
            assert_eq!(writes.take(), None);
        }
    }
}
