//! The Capture screen's state: the ring of frames, the filter, the
//! selection and its decode, and the keys.
//!
//! The list follows the newest frame until it is paused: Space, or moving
//! the selection, freezes it at the frame number it had reached. Frames
//! still arrive into the ring while it is paused (the title counts them),
//! and Space follows again. Only the selected frame is decoded in full,
//! once, in [`App::settle`].

use std::cell::{Cell, RefCell};
use std::time::UNIX_EPOCH;

use ratatui::text::Text;
use tui_tree_widget::{TreeItem, TreeState};

use super::devices::{DeviceTable, Movement};
use super::watch::Screen;
use super::{App, FilterKey, Link, FLASH_FOR};
use crate::decode::tree::{decode_tree, Field};
use crate::tui::capture::filter::Filter;
use crate::tui::capture::ring::Ring;
use crate::tui::capture::{CaptureCommand, CaptureEvent, ExportJob, DEFAULT_RING};
use crate::tui::message::Command;

/// The focused pane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Focus {
    /// The frame list.
    #[default]
    List,
    /// The decode tree.
    Tree,
    /// The hex view (it follows the tree's selection).
    Hex,
}

/// A key on the Capture screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureKey {
    /// Move in the focused pane.
    Move(Movement),
    /// Next pane, or the previous one.
    Cycle {
        /// Backwards.
        back: bool,
    },
    /// Enter: from the list, the tree; in the tree, open or close a node.
    Toggle,
    /// Left in the tree: close the node, or go to its parent.
    Collapse,
    /// Right in the tree: open the node.
    Expand,
    /// Space: pause, or follow again.
    Pause,
    /// `z`: forget every frame.
    Clear,
    /// `e`: export the frames the filter keeps.
    Export {
        /// B/IP as raw IPv4 rather than Ethernet.
        raw_ipv4: bool,
    },
    /// Esc: clear the filter, then go back.
    Back,
}

/// The selected frame, decoded.
pub(crate) struct Detail {
    /// Its number.
    pub(crate) number: u64,
    /// Its fields.
    pub(crate) fields: Vec<Field>,
    /// The same, as the tree widget draws them.
    pub(crate) items: Vec<TreeItem<'static, usize>>,
}

/// The Capture screen.
pub(crate) struct CaptureState {
    /// The frames.
    pub(crate) ring: Ring,
    /// The filter as typed.
    pub(crate) filter: String,
    /// The user has opened the screen: each session captures from its start.
    pub(crate) wanted: bool,
    /// What the running sources capture, and whether each is the session's.
    pub(crate) sources: Vec<(String, bool)>,
    /// Why the last source ended.
    pub(crate) ended: Option<String>,
    /// Frozen at this frame number; following when `None`.
    pub(crate) paused: Option<u64>,
    selected: Option<u64>,
    /// The focused pane.
    pub(crate) focus: Focus,
    /// Frames lost before reaching the ring.
    pub(crate) lost: u64,
    /// The selected frame's decode.
    pub(crate) detail: Option<Detail>,
    /// The tree's open nodes, selection and scroll.
    pub(crate) tree: RefCell<TreeState<usize>>,
    /// The list's first row, kept between frames.
    pub(crate) offset: Cell<usize>,
    /// Rows the list showed last; a page.
    pub(crate) page: Cell<usize>,
    /// Frames decoded in full, for the test that only the selected one is.
    pub(crate) decodes: u64,
}

impl CaptureState {
    /// An empty screen with a ring of `capacity` frames.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            ring: Ring::new(capacity),
            filter: String::new(),
            wanted: false,
            sources: Vec::new(),
            ended: None,
            paused: None,
            selected: None,
            focus: Focus::List,
            lost: 0,
            detail: None,
            tree: RefCell::new(TreeState::default()),
            offset: Cell::new(0),
            page: Cell::new(10),
            decodes: 0,
        }
    }

    /// Numbers of the frames the list shows, oldest first: those the filter
    /// keeps, up to where it was paused.
    pub(crate) fn visible(&self) -> impl DoubleEndedIterator<Item = u64> + ExactSizeIterator + '_ {
        self.ring.kept().range(..self.shown()).copied()
    }

    /// How many frames the list shows.
    pub(crate) fn shown(&self) -> usize {
        let kept = self.ring.kept();
        match self.paused {
            Some(at) => kept.partition_point(|n| *n <= at),
            None => kept.len(),
        }
    }

    /// The number of the list's row `index`.
    pub(crate) fn row(&self, index: usize) -> Option<u64> {
        (index < self.shown())
            .then(|| self.ring.kept().get(index).copied())
            .flatten()
    }

    /// The list row of frame `number`.
    pub(crate) fn index_of(&self, number: u64) -> Option<usize> {
        let index = self.ring.kept().binary_search(&number).ok()?;
        (index < self.shown()).then_some(index)
    }

    /// Frames the filter keeps that arrived while paused.
    pub(crate) fn held_back(&self) -> usize {
        self.ring.kept().len() - self.shown()
    }

    /// Stop following, at the newest frame shown, keeping the selection.
    fn pause(&mut self) {
        if self.paused.is_none() {
            let newest = self.visible().next_back();
            self.selected = newest;
            self.paused = Some(self.ring.next() - 1);
        }
    }

    /// The selected frame's number: the newest shown while following, or
    /// when the one selected has left the list (evicted, or filtered out).
    pub(crate) fn selected(&self) -> Option<u64> {
        let kept = self
            .selected
            .filter(|n| self.paused.is_some() && self.index_of(*n).is_some());
        kept.or_else(|| self.visible().next_back())
    }

    /// The field the tree has selected, whose octets the hex view marks.
    pub(crate) fn selected_field(&self) -> Option<&Field> {
        let detail = self.detail.as_ref()?;
        let tree = self.tree.borrow();
        let mut path = tree.selected().iter();
        let mut field = detail.fields.get(*path.next()?)?;
        for index in path {
            field = field.children.get(*index)?;
        }
        Some(field)
    }

    /// Decode the selected frame if it changed. Called before drawing.
    pub(crate) fn settle(&mut self) {
        let selected = self.selected();
        if self.detail.as_ref().map(|d| d.number) == selected {
            return;
        }
        self.detail = selected.and_then(|number| {
            let frame = self.ring.get(number)?;
            let fields = decode_tree(frame.framing, &frame.bytes);
            let items = items(&fields);
            Some(Detail {
                number,
                fields,
                items,
            })
        });
        self.decodes += u64::from(self.detail.is_some());
        // A new frame opens with every node open and the first selected.
        let mut tree = TreeState::default();
        if let Some(detail) = &self.detail {
            open_all(&mut tree, &detail.fields, &mut Vec::new());
            tree.select(vec![0]);
        }
        *self.tree.get_mut() = tree;
    }

    fn move_list(&mut self, movement: Movement) {
        let Some(last) = self.shown().checked_sub(1) else {
            return;
        };
        // Moving pauses, so the selection stays put under new frames.
        let current = self.selected();
        self.paused.get_or_insert(self.ring.next() - 1);
        let index = current.and_then(|n| self.index_of(n)).unwrap_or(last);
        let page = self.page.get().max(1);
        let next = match movement {
            Movement::Up => index.saturating_sub(1),
            Movement::Down => (index + 1).min(last),
            Movement::PageUp => index.saturating_sub(page),
            Movement::PageDown => (index + page).min(last),
            Movement::Home => 0,
            Movement::End => last,
        };
        self.selected = self.row(next);
    }

    fn move_tree(&mut self, movement: Movement) {
        let Some(detail) = &self.detail else {
            return;
        };
        let tree = self.tree.get_mut();
        let flat: Vec<Vec<usize>> = tree
            .flatten(&detail.items)
            .into_iter()
            .map(|f| f.identifier)
            .collect();
        let Some(last) = flat.len().checked_sub(1) else {
            return;
        };
        let index = flat
            .iter()
            .position(|id| id == tree.selected())
            .unwrap_or(0);
        let next = match movement {
            Movement::Up => index.saturating_sub(1),
            Movement::Down => (index + 1).min(last),
            Movement::PageUp => index.saturating_sub(10),
            Movement::PageDown => (index + 10).min(last),
            Movement::Home => 0,
            Movement::End => last,
        };
        tree.select(flat[next].clone());
    }

    /// Apply a worker report; returns a message for the footer.
    pub(crate) fn event(&mut self, event: CaptureEvent) -> Option<String> {
        match event {
            CaptureEvent::Started { what, session } => {
                self.ended = None;
                self.sources.push((what, session));
                None
            }
            CaptureEvent::Frames(batch) => {
                self.lost += batch.lost;
                for frame in batch.frames {
                    self.ring.push(frame);
                }
                None
            }
            CaptureEvent::Ended { what, why } => {
                // Only that source: a session ending leaves libpcap running.
                self.sources.retain(|(source, _)| *source != what);
                let text = format!("Capture from {what}: {why}.");
                self.ended = Some(format!("{what}: {why}"));
                Some(text)
            }
            CaptureEvent::Exported(exported) => {
                let written = exported.written.join(", ");
                Some(match (exported.written.is_empty(), exported.error) {
                    (false, None) => format!("Exported {written}."),
                    (true, Some(error)) => format!("Export failed: {error}"),
                    (false, Some(error)) => {
                        format!("Exported {written}, then failed: {error}")
                    }
                    (true, None) => "Nothing exported.".into(),
                })
            }
        }
    }
}

impl Default for CaptureState {
    fn default() -> Self {
        Self::new(DEFAULT_RING)
    }
}

/// The tree widget's items for `fields`, identified by position.
fn items(fields: &[Field]) -> Vec<TreeItem<'static, usize>> {
    fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let text = Text::raw(field.label.clone());
            if field.children.is_empty() {
                TreeItem::new_leaf(index, text)
            } else {
                // Positions are distinct, so this can't fail.
                TreeItem::new(index, text.clone(), items(&field.children))
                    .unwrap_or_else(|_| TreeItem::new_leaf(index, text))
            }
        })
        .collect()
}

fn open_all(tree: &mut TreeState<usize>, fields: &[Field], path: &mut Vec<usize>) {
    for (index, field) in fields.iter().enumerate() {
        if !field.children.is_empty() {
            path.push(index);
            tree.open(path.clone());
            open_all(tree, &field.children, path);
            path.pop();
        }
    }
}

impl App {
    /// Show the Capture screen, starting a capture the first time.
    pub(crate) fn show_capture(&mut self, out: &mut Vec<Command>) {
        self.screen = Screen::Capture;
        if !self.capture.wanted {
            self.capture.wanted = true;
            self.start_capture(out);
        }
    }

    /// Ask the session for a capture, when one is up to give it.
    pub(crate) fn start_capture(&mut self, out: &mut Vec<Command>) {
        let monitoring = self.link == Link::Passive && self.monitor.is_some();
        if self.capture.wanted && (matches!(self.link, Link::Up { .. }) || monitoring) {
            out.push(Command::Capture(CaptureCommand::Start));
        }
    }

    /// A key on the Capture screen.
    pub(crate) fn capture_key(&mut self, key: CaptureKey, out: &mut Vec<Command>) {
        let capture = &mut self.capture;
        match key {
            CaptureKey::Move(movement) if capture.focus == Focus::List => {
                capture.move_list(movement);
            }
            CaptureKey::Move(movement) => capture.move_tree(movement),
            CaptureKey::Cycle { back } => {
                capture.focus = match (capture.focus, back) {
                    (Focus::List, false) | (Focus::Hex, true) => Focus::Tree,
                    (Focus::Tree, false) | (Focus::List, true) => Focus::Hex,
                    (Focus::Hex, false) | (Focus::Tree, true) => Focus::List,
                };
                // Reading a frame's fields pauses, so a new frame doesn't
                // replace the tree under the cursor.
                if capture.focus != Focus::List {
                    capture.pause();
                }
            }
            CaptureKey::Toggle if capture.focus == Focus::List => {
                capture.focus = Focus::Tree;
                capture.pause();
            }
            CaptureKey::Toggle => {
                capture.tree.get_mut().toggle_selected();
            }
            CaptureKey::Collapse => {
                let tree = capture.tree.get_mut();
                let selected = tree.selected().to_vec();
                if !tree.close(&selected) && selected.len() > 1 {
                    tree.select(selected[..selected.len() - 1].to_vec());
                }
            }
            CaptureKey::Expand => {
                let tree = capture.tree.get_mut();
                let selected = tree.selected().to_vec();
                tree.open(selected);
            }
            CaptureKey::Pause if capture.paused.is_some() => capture.paused = None,
            CaptureKey::Pause => capture.pause(),
            CaptureKey::Clear => {
                capture.ring.clear();
                capture.paused = None;
                capture.selected = None;
                capture.lost = 0;
                self.flash("Cleared the capture.", FLASH_FOR);
            }
            CaptureKey::Export { raw_ipv4 } => self.export_capture(raw_ipv4, out),
            CaptureKey::Back if !capture.filter.is_empty() => {
                capture.filter.clear();
                capture.ring.set_filter(Filter::default());
            }
            CaptureKey::Back => self.screen = Screen::Main,
        }
    }

    /// A key on the filter line, on the Capture screen. The filter applies
    /// when the line is closed with Enter.
    pub(crate) fn capture_filter_key(&mut self, key: FilterKey) {
        match key {
            FilterKey::Char(ch) => self.capture.filter.push(ch),
            FilterKey::Backspace => {
                self.capture.filter.pop();
            }
            FilterKey::Commit => {
                self.filter_editing = false;
                match Filter::parse(&self.capture.filter) {
                    Ok(mut filter) => {
                        let devices = &self.devices;
                        filter.resolve(|instance| device_addresses(devices, instance));
                        self.capture.ring.set_filter(filter);
                    }
                    Err(error) => self.flash(error, FLASH_FOR),
                }
            }
            FilterKey::Clear => {
                self.filter_editing = false;
                self.capture.filter.clear();
                self.capture.ring.set_filter(Filter::default());
            }
        }
    }

    /// Hand the frames the filter keeps to the coordinator to write.
    pub(crate) fn export_capture(&mut self, raw_ipv4: bool, out: &mut Vec<Command>) {
        let capture = &self.capture;
        let frames: Vec<_> = capture
            .visible()
            .filter_map(|n| capture.ring.shared(n).cloned())
            .collect();
        let Some(last) = frames.last() else {
            self.flash("Nothing to export.", FLASH_FOR);
            return;
        };
        let since = last.time.duration_since(UNIX_EPOCH).unwrap_or_default();
        let time = crate::tui::safety::audit::utc(since.as_secs(), since.subsec_millis());
        let stem = format!("capture-{}", time.replace(':', "-"));
        self.flash(format!("Exporting {} frames...", frames.len()), FLASH_FOR);
        out.push(Command::CaptureExport(Box::new(ExportJob {
            frames,
            raw_ipv4,
            stem,
        })));
    }

    /// Look the filter's `dev:` terms up again after the device table
    /// changed, and filter the ring again only when an address did.
    pub(crate) fn resolve_capture_devices(&mut self) {
        if !self.capture.ring.filter().has_devices() {
            return;
        }
        let mut filter = self.capture.ring.filter().clone();
        let devices = &self.devices;
        if filter.resolve(|instance| device_addresses(devices, instance)) {
            self.capture.ring.set_filter(filter);
        }
    }
}

/// Where device `instance` is, as a captured frame would name it: the
/// table's address, which for a routed device is its routed station
/// (`2:0x05`) in the same syntax the capture writes.
fn device_addresses(devices: &DeviceTable, instance: u32) -> Vec<String> {
    devices
        .get(instance)
        .map(|row| row.address.clone())
        .into_iter()
        .collect()
}
