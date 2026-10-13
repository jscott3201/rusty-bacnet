//! The shared half-duplex medium and the receivers' character framing.
//!
//! Every driver puts octets on the line back to back, each occupying ten bit
//! times. Propagation is ideal, so every receiver sees the same line, and the
//! line keeps one receiver framing window for all of them, the way a UART
//! frames characters:
//!
//! - An idle receiver starts a character at the first octet that begins.
//! - The character ends when that octet ends. If no other octet was on the
//!   line in between, receivers get the octet (with any injected corruption);
//!   otherwise they get a framing error.
//! - If an octet is still on the line when a character ends (two drivers out
//!   of step), the receiver starts another, garbled, character at once.
//!
//! So a clean line delivers each octet once, and a collision delivers about
//! one garbled character per octet time while it lasts.

use std::collections::BTreeMap;

use bacnet_mstp_core::Instant;

use crate::spec::Corruption;
use crate::wire::WireTiming;

/// Identifies one transmission on the line.
pub(crate) type TxId = u64;

/// Who drives a transmission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Driver {
    /// A node, master or scripted, by station address.
    Station(u8),
    /// An injected noise burst, numbered in injection order.
    Noise(u64),
}

/// A transmission in progress.
#[derive(Debug, Clone)]
pub(crate) struct Transmission {
    pub driver: Driver,
    pub octets: Vec<u8>,
    pub start: Instant,
    /// Octets started so far.
    pub started: usize,
    pub truncated: bool,
    pub collided: bool,
    pub corruption: Option<(usize, Corruption)>,
}

#[derive(Debug, Clone, Copy)]
struct ActiveOctet {
    tx: TxId,
    driver: Driver,
    end: Instant,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    start: Instant,
    end: Instant,
    /// The transmission and octet index that opened the window in step, if
    /// one did.
    opener: Option<(TxId, usize)>,
    corrupted: bool,
}

/// What receivers get when a character ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Character {
    Clean(u8),
    Garbled,
}

/// The result of ending the current receiver window.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WindowOutcome {
    pub start: Instant,
    pub character: Character,
    /// The driver whose octet opened the window in step.
    pub opener: Option<Driver>,
    /// When a garbled follow-on window ends, if one opened.
    pub reopened: Option<Instant>,
}

/// The result of starting an octet.
#[derive(Debug, Clone)]
pub(crate) struct OctetOutcome {
    /// Other drivers whose octets overlap this one.
    pub overlaps: Vec<Driver>,
    /// When the receiver window this octet opened ends, if it opened one.
    pub window_end: Option<Instant>,
}

/// What follows an octet of a transmission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Next {
    Octet { at: Instant, index: usize },
    End { at: Instant },
}

#[derive(Debug)]
pub(crate) struct Line {
    timing: WireTiming,
    next_id: TxId,
    live: BTreeMap<TxId, Transmission>,
    active: Vec<ActiveOctet>,
    window: Option<Window>,
    last_activity: Option<Instant>,
}

impl Line {
    pub(crate) fn new(timing: WireTiming) -> Self {
        Self {
            timing,
            next_id: 0,
            live: BTreeMap::new(),
            active: Vec::new(),
            window: None,
            last_activity: None,
        }
    }

    /// When receivers last got a character, clean or garbled: the line
    /// activity a node's turnaround is measured from.
    pub(crate) fn last_activity(&self) -> Option<Instant> {
        self.last_activity
    }

    /// Whether any octet is on the line at `now`.
    pub(crate) fn busy(&self, now: Instant) -> bool {
        self.active.iter().any(|octet| octet.end > now)
    }

    /// Transmissions in progress.
    pub(crate) fn live_count(&self) -> usize {
        self.live.len()
    }

    pub(crate) fn get(&self, tx: TxId) -> Option<&Transmission> {
        self.live.get(&tx)
    }

    /// Register a transmission. Its first octet starts with
    /// [`Self::start_octet`].
    pub(crate) fn begin(
        &mut self,
        driver: Driver,
        octets: Vec<u8>,
        start: Instant,
        corruption: Option<(usize, Corruption)>,
    ) -> TxId {
        let id = self.next_id;
        self.next_id += 1;
        self.live.insert(
            id,
            Transmission {
                driver,
                octets,
                start,
                started: 0,
                truncated: false,
                collided: false,
                corruption,
            },
        );
        id
    }

    /// Put octet `index` of `tx` on the line at `now`.
    pub(crate) fn start_octet(&mut self, tx: TxId, index: usize, now: Instant) -> OctetOutcome {
        self.prune(now);
        let (driver, end) = {
            let transmission = self.live.get_mut(&tx).expect("a live transmission");
            transmission.started = index + 1;
            (
                transmission.driver,
                transmission.start + self.timing.boundary(index + 1),
            )
        };
        let mut overlaps = Vec::new();
        for octet in &self.active {
            if octet.driver != driver && octet.end > now {
                overlaps.push((octet.tx, octet.driver));
            }
        }
        if !overlaps.is_empty() {
            if let Some(transmission) = self.live.get_mut(&tx) {
                transmission.collided = true;
            }
            for (other, _) in &overlaps {
                if let Some(transmission) = self.live.get_mut(other) {
                    transmission.collided = true;
                }
            }
        }
        self.active.push(ActiveOctet { tx, driver, end });
        let window_end = match self.window.as_mut() {
            None => {
                self.window = Some(Window {
                    start: now,
                    end,
                    opener: Some((tx, index)),
                    corrupted: false,
                });
                Some(end)
            }
            Some(window) => {
                window.corrupted = true;
                None
            }
        };
        let mut drivers: Vec<Driver> = overlaps.into_iter().map(|(_, driver)| driver).collect();
        drivers.sort_unstable();
        drivers.dedup();
        OctetOutcome {
            overlaps: drivers,
            window_end,
        }
    }

    /// What comes after octet `index` of `tx`.
    pub(crate) fn next_after(&self, tx: TxId, index: usize) -> Option<Next> {
        let transmission = self.live.get(&tx)?;
        let following = index + 1;
        let at = transmission.start + self.timing.boundary(following);
        Some(if following < transmission.octets.len() {
            Next::Octet {
                at,
                index: following,
            }
        } else {
            Next::End { at }
        })
    }

    /// End the receiver window that falls due at `now`.
    pub(crate) fn end_window(&mut self, now: Instant) -> Option<WindowOutcome> {
        let window = self.window.take()?;
        let mut opener = None;
        let character = match window.opener {
            Some((tx, index)) => match self.live.get(&tx) {
                Some(transmission) => {
                    opener = Some(transmission.driver);
                    if window.corrupted || transmission.truncated {
                        Character::Garbled
                    } else {
                        let byte = transmission.octets[index];
                        match transmission.corruption {
                            Some((at, Corruption::Xor(mask))) if at == index => {
                                Character::Clean(byte ^ mask)
                            }
                            Some((at, Corruption::FramingError)) if at == index => {
                                Character::Garbled
                            }
                            _ => Character::Clean(byte),
                        }
                    }
                }
                None => Character::Garbled,
            },
            None => Character::Garbled,
        };
        self.last_activity = Some(now);
        self.prune(now);
        let reopened = if self.busy(now) {
            let end = now + self.timing.octet();
            self.window = Some(Window {
                start: now,
                end,
                opener: None,
                corrupted: true,
            });
            Some(end)
        } else {
            None
        };
        Some(WindowOutcome {
            start: window.start,
            character,
            opener,
            reopened,
        })
    }

    /// Cut a transmission off at `now` (its driver lost power). Its octet
    /// on the line, if any, reaches receivers as a garbled character.
    pub(crate) fn truncate(&mut self, tx: TxId, now: Instant) -> Option<Transmission> {
        let mut transmission = self.live.remove(&tx)?;
        transmission.truncated = true;
        self.active.retain(|octet| octet.tx != tx);
        if let Some(window) = self.window.as_mut() {
            if window.opener.is_some_and(|(opener, _)| opener == tx) && window.end > now {
                window.corrupted = true;
            }
        }
        Some(transmission)
    }

    /// Remove a transmission whose last octet has ended.
    pub(crate) fn finish(&mut self, tx: TxId) -> Option<Transmission> {
        self.live.remove(&tx)
    }

    fn prune(&mut self, now: Instant) {
        self.active.retain(|octet| octet.end > now);
    }
}

#[cfg(test)]
mod tests {
    use super::{Character, Driver, Line, Next};
    use crate::spec::Corruption;
    use crate::wire::WireTiming;
    use bacnet_mstp_core::Instant;

    fn us(value: u64) -> Instant {
        Instant::from_micros(value)
    }

    #[test]
    fn a_lone_driver_delivers_each_octet_once() {
        let timing = WireTiming::new(9_600);
        let mut line = Line::new(timing);
        let tx = line.begin(Driver::Station(1), vec![0x55, 0xFF], us(0), None);
        let first = line.start_octet(tx, 0, us(0));
        assert!(first.overlaps.is_empty());
        assert_eq!(first.window_end, Some(us(1_042)));
        let outcome = line.end_window(us(1_042)).unwrap();
        assert_eq!(outcome.character, Character::Clean(0x55));
        assert_eq!(outcome.reopened, None);
        assert_eq!(
            line.next_after(tx, 0),
            Some(Next::Octet {
                at: us(1_042),
                index: 1
            })
        );
        line.start_octet(tx, 1, us(1_042));
        assert_eq!(line.next_after(tx, 1), Some(Next::End { at: us(2_084) }));
        let outcome = line.end_window(us(2_084)).unwrap();
        assert_eq!(outcome.character, Character::Clean(0xFF));
        assert!(!line.finish(tx).unwrap().collided);
        assert_eq!(line.last_activity(), Some(us(2_084)));
    }

    #[test]
    fn overlapping_drivers_garble_and_mark_both() {
        let timing = WireTiming::new(9_600);
        let mut line = Line::new(timing);
        let a = line.begin(Driver::Station(1), vec![0x55, 0xFF], us(0), None);
        let b = line.begin(Driver::Station(2), vec![0x55], us(500), None);
        line.start_octet(a, 0, us(0));
        let overlap = line.start_octet(b, 0, us(500));
        assert_eq!(overlap.overlaps, vec![Driver::Station(1)]);
        assert_eq!(overlap.window_end, None);
        let outcome = line.end_window(us(1_042)).unwrap();
        assert_eq!(outcome.character, Character::Garbled);
        // Station 2's octet is still on the line: a garbled follow-on window.
        assert_eq!(outcome.reopened, Some(us(2_084)));
        assert!(line.get(a).unwrap().collided);
        assert!(line.get(b).unwrap().collided);
    }

    #[test]
    fn corruption_and_truncation_reach_receivers() {
        let timing = WireTiming::new(9_600);
        let mut line = Line::new(timing);
        let tx = line.begin(
            Driver::Station(1),
            vec![0x55, 0xFF],
            us(0),
            Some((1, Corruption::Xor(0x01))),
        );
        line.start_octet(tx, 0, us(0));
        assert_eq!(
            line.end_window(us(1_042)).unwrap().character,
            Character::Clean(0x55)
        );
        line.start_octet(tx, 1, us(1_042));
        assert_eq!(
            line.end_window(us(2_084)).unwrap().character,
            Character::Clean(0xFE)
        );

        let cut = line.begin(Driver::Station(2), vec![0x55, 0xFF], us(10_000), None);
        line.start_octet(cut, 0, us(10_000));
        assert!(line.truncate(cut, us(10_500)).unwrap().truncated);
        assert!(!line.busy(us(10_500)));
        assert_eq!(
            line.end_window(us(11_042)).unwrap().character,
            Character::Garbled
        );
    }
}
