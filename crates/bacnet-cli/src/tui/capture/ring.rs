//! The bounded store of captured frames.
//!
//! Frames get consecutive numbers as they arrive, so the one at number `n`
//! is at index `n - first` and a lookup costs nothing. The numbers of the
//! frames the filter keeps are a second queue in the same order: a new
//! frame is tested once, and the oldest leave both queues together. Only a
//! new filter walks the whole ring.

use std::collections::VecDeque;
use std::sync::Arc;

use super::filter::Filter;
use super::CaptureFrame;

/// Frames, oldest first, at most `capacity` of them.
#[derive(Debug)]
pub(crate) struct Ring {
    /// Shared, so an export takes them without copying their octets.
    frames: VecDeque<Arc<CaptureFrame>>,
    /// The number of `frames[0]`.
    first: u64,
    capacity: usize,
    /// Numbers of the frames `filter` keeps, ascending.
    kept: VecDeque<u64>,
    filter: Filter,
}

impl Ring {
    /// An empty ring of `capacity` frames (at least one).
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            first: 1,
            capacity: capacity.max(1),
            kept: VecDeque::new(),
            filter: Filter::default(),
        }
    }

    /// Store a frame, dropping the oldest when full; returns its number.
    pub(crate) fn push(&mut self, frame: CaptureFrame) -> u64 {
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
            if self.kept.front() == Some(&self.first) {
                self.kept.pop_front();
            }
            self.first += 1;
        }
        let number = self.next();
        if self.filter.matches(&frame) {
            self.kept.push_back(number);
        }
        self.frames.push_back(Arc::new(frame));
        number
    }

    /// The number the next frame gets.
    pub(crate) fn next(&self) -> u64 {
        self.first + self.frames.len() as u64
    }

    /// Frames held.
    pub(crate) fn len(&self) -> usize {
        self.frames.len()
    }

    /// The most it holds.
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    /// Frame `number`, if still held.
    pub(crate) fn get(&self, number: u64) -> Option<&CaptureFrame> {
        self.shared(number).map(|frame| &**frame)
    }

    /// Frame `number` as the ring holds it, shared.
    pub(crate) fn shared(&self, number: u64) -> Option<&Arc<CaptureFrame>> {
        let index = number.checked_sub(self.first)?;
        self.frames.get(usize::try_from(index).ok()?)
    }

    /// Numbers of the frames the filter keeps, oldest first.
    pub(crate) fn kept(&self) -> &VecDeque<u64> {
        &self.kept
    }

    /// The filter applied.
    pub(crate) fn filter(&self) -> &Filter {
        &self.filter
    }

    /// Apply a new filter to every frame held.
    pub(crate) fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        let first = self.first;
        self.kept = self
            .frames
            .iter()
            .zip(first..)
            .filter(|(frame, _)| self.filter.matches(frame))
            .map(|(_, number)| number)
            .collect();
    }

    /// Forget every frame; numbering goes on.
    pub(crate) fn clear(&mut self) {
        self.first = self.next();
        self.frames.clear();
        self.kept.clear();
    }
}
