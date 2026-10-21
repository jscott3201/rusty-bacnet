//! The device browser (point checkout): the device card, its object list,
//! and one object's properties and priority array. Read-only.
//!
//! Every read the browser asks for carries a fresh request id. The browser
//! remembers the id of the device read and of the latest object read, and
//! drops any result that answers neither, so a slow answer to a request the
//! user has moved on from never replaces a newer one.

use std::cell::Cell;

use bacnet_types::primitives::ObjectIdentifier;

use super::devices::Movement;
use super::objects::ObjectList;
use super::watch::Screen;
use super::{App, Link, FLASH_FOR};
use crate::tui::message::Command;
use crate::tui::point::{BrowseRead, BrowseResult, DeviceCard, ObjectView, ReqId};

/// A pane of the browser. Below the full width only the focused one is
/// drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    /// The device card.
    Card,
    /// The object list.
    Objects,
    /// The selected object's properties.
    Properties,
    /// A commandable object's priority array (#1894). Below the full
    /// width it is a pane of its own, so the properties keep the width;
    /// from it, the pane beside the properties takes the accent.
    Priority,
}

/// Progress of the device read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListState {
    /// Reading the device card.
    Card,
    /// Reading the Object_List element by element.
    Reading {
        /// Elements read.
        done: u32,
        /// Elements in the list.
        total: u32,
    },
    /// Read.
    Ready {
        /// It took element-by-element reads.
        indexed: bool,
    },
    /// Read element by element up to an element that failed; the list
    /// holds the ones before it.
    Partial(String),
    /// The read failed.
    Failed(String),
}

/// The properties pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ViewState {
    /// No object opened yet.
    Empty,
    /// Reading this object.
    Reading(ObjectIdentifier),
    /// Read.
    Ready(Box<ObjectView>),
    /// Reading this object failed.
    Failed(ObjectIdentifier, String),
}

/// The browser's state.
pub(crate) struct Browser {
    /// The device instance.
    pub(crate) device: u32,
    /// Where it answered from, for the title.
    pub(crate) address: String,
    /// The device card, once read.
    pub(crate) card: Option<DeviceCard>,
    /// The device read's progress.
    pub(crate) list: ListState,
    /// The object list.
    pub(crate) objects: ObjectList,
    /// The properties pane.
    pub(crate) view: ViewState,
    /// The focused pane.
    pub(crate) focus: Pane,
    /// First visible property line, kept by the view between frames.
    pub(crate) scroll: Cell<usize>,
    device_req: ReqId,
    object_req: Option<ReqId>,
}

impl Browser {
    fn new(device: u32, address: String, device_req: ReqId) -> Self {
        Self {
            device,
            address,
            card: None,
            list: ListState::Card,
            objects: ObjectList::default(),
            view: ViewState::Empty,
            focus: Pane::Objects,
            scroll: Cell::new(0),
            device_req,
            object_req: None,
        }
    }

    /// The selected object, which `a` adds to the watch list.
    pub(crate) fn selected_point(&self) -> Option<(u32, ObjectIdentifier)> {
        self.objects.selected().map(|object| (self.device, object))
    }

    /// Apply a result for request `req`; one for any other request is
    /// stale and dropped. Returns whether it was applied.
    pub(crate) fn apply(&mut self, req: ReqId, result: BrowseResult) -> bool {
        if req == self.device_req {
            match result {
                BrowseResult::Card(card) => self.card = Some(card),
                BrowseResult::Progress { done, total } => {
                    self.list = ListState::Reading { done, total };
                }
                BrowseResult::Objects {
                    objects,
                    indexed,
                    stopped,
                } => {
                    self.objects.set(objects);
                    self.list = match stopped {
                        Some(error) => ListState::Partial(error),
                        None => ListState::Ready { indexed },
                    };
                }
                BrowseResult::Failed(error) => self.list = ListState::Failed(error),
                BrowseResult::Object(_) | BrowseResult::Names { .. } => return false,
            }
            return true;
        }
        if Some(req) != self.object_req {
            return false;
        }
        let object = match &self.view {
            ViewState::Reading(object) => *object,
            _ => return false,
        };
        self.view = match result {
            BrowseResult::Object(view) => ViewState::Ready(view),
            BrowseResult::Failed(error) => ViewState::Failed(object, error),
            _ => return false,
        };
        self.object_req = None;
        true
    }

    /// Mark `object` as being read again by request `req`.
    pub(crate) fn reading(&mut self, req: ReqId, object: ObjectIdentifier) {
        self.object_req = Some(req);
        self.view = ViewState::Reading(object);
    }

    /// Focus the next pane, or the previous one with `back`.
    pub(crate) fn cycle_focus(&mut self, back: bool) {
        let order = self.panes();
        let at = order
            .iter()
            .position(|pane| *pane == self.shown_focus())
            .unwrap_or(1);
        let next = if back { at + order.len() - 1 } else { at + 1 };
        self.focus = order[next % order.len()];
    }

    /// The panes Tab visits: the priority array only for an object that
    /// has one.
    pub(crate) fn panes(&self) -> &'static [Pane] {
        const ALL: [Pane; 4] = [Pane::Card, Pane::Objects, Pane::Properties, Pane::Priority];
        if self.has_priority() {
            &ALL
        } else {
            &ALL[..3]
        }
    }

    /// The focused pane as drawn: the priority pane falls back to the
    /// properties once the object shown has no priority array.
    pub(crate) fn shown_focus(&self) -> Pane {
        match self.focus {
            Pane::Priority if !self.has_priority() => Pane::Properties,
            focus => focus,
        }
    }

    fn has_priority(&self) -> bool {
        matches!(&self.view, ViewState::Ready(view) if view.priority.is_some())
    }

    /// Move in the focused pane.
    pub(crate) fn move_selection(&mut self, movement: Movement) {
        match self.focus {
            Pane::Objects => self.objects.move_selection(movement),
            Pane::Properties => {
                let at = self.scroll.get();
                self.scroll.set(match movement {
                    Movement::Up => at.saturating_sub(1),
                    Movement::Down => at.saturating_add(1),
                    Movement::PageUp => at.saturating_sub(10),
                    Movement::PageDown => at.saturating_add(10),
                    Movement::Home => 0,
                    // The view clamps it to the last line.
                    Movement::End => usize::MAX,
                });
            }
            Pane::Card | Pane::Priority => {}
        }
    }
}

impl App {
    pub(super) fn next_req(&mut self) -> ReqId {
        self.next_req += 1;
        self.next_req
    }

    /// Enter: open the selected device, or read the selected object.
    pub(super) fn open(&mut self, out: &mut Vec<Command>) {
        if !self.may_transmit() {
            return;
        }
        if !matches!(self.link, Link::Up { .. }) {
            self.flash("Not connected.", FLASH_FOR);
            return;
        }
        if self.screen != Screen::Browse {
            self.settle_devices();
            let Some(row) = self.devices.selected_row() else {
                self.flash("No device selected. Press d to send a Who-Is.", FLASH_FOR);
                return;
            };
            let (device, address) = (row.instance, row.address.clone());
            self.filter_editing = false;
            // The device kept from before is shown again as it was, at the
            // address it answers from now.
            if let Some(kept) = self.browser.as_deref_mut().filter(|b| b.device == device) {
                kept.address = address;
                self.screen = Screen::Browse;
                return;
            }
            let req = self.next_req();
            self.close_browser(out);
            self.screen = Screen::Browse;
            self.browser = Some(Box::new(Browser::new(device, address, req)));
            out.push(Command::Browse {
                req,
                device,
                read: BrowseRead::Device,
            });
            return;
        }
        let req = self.next_req();
        match self.browser.as_deref_mut() {
            None => {}
            Some(browser) if browser.focus == Pane::Objects => {
                if let Some(object) = browser.objects.selected() {
                    browser.object_req = Some(req);
                    browser.view = ViewState::Reading(object);
                    browser.focus = Pane::Properties;
                    browser.scroll.set(0);
                    out.push(Command::Browse {
                        req,
                        device: browser.device,
                        read: BrowseRead::Object(object),
                    });
                }
            }
            Some(_) => {}
        }
    }

    /// `r`: read the open object again, or the whole device from the card
    /// or the object list.
    pub(super) fn refresh(&mut self, out: &mut Vec<Command>) {
        if !matches!(self.link, Link::Up { .. }) || !self.may_transmit() {
            return;
        }
        let req = self.next_req();
        let Some(browser) = self.shown_browser_mut() else {
            return;
        };
        let reading = match (&browser.view, browser.shown_focus()) {
            (ViewState::Ready(view), Pane::Properties | Pane::Priority) => Some(view.object),
            (ViewState::Failed(object, _) | ViewState::Reading(object), Pane::Properties) => {
                Some(*object)
            }
            _ => None,
        };
        let read = match reading {
            Some(object) => {
                browser.object_req = Some(req);
                browser.view = ViewState::Reading(object);
                BrowseRead::Object(object)
            }
            None => {
                browser.device_req = req;
                browser.list = ListState::Card;
                // A device read replaces any object read still waiting for
                // the device, and that one never answers.
                if let ViewState::Reading(_) = browser.view {
                    browser.view = ViewState::Empty;
                    browser.object_req = None;
                }
                BrowseRead::Device
            }
        };
        out.push(Command::Browse {
            req,
            device: browser.device,
            read,
        });
    }

    /// Leave the browser for the device table, stopping its reads.
    pub(super) fn close_browser(&mut self, out: &mut Vec<Command>) {
        if self.screen == Screen::Browse {
            self.screen = Screen::Main;
        }
        if let Some(browser) = self.browser.take() {
            self.filter_editing = false;
            out.push(Command::StopBrowse {
                device: browser.device,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ObjectType;

    use super::*;
    use crate::tui::point::ReadVia;

    fn view(instance: u32) -> BrowseResult {
        BrowseResult::Object(Box::new(ObjectView {
            object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
            properties: Vec::new(),
            health: Default::default(),
            priority: None,
            via: ReadVia::Rpm,
            name: None,
        }))
    }

    /// Only the latest object read and the current device read apply; a
    /// late answer to an earlier request is dropped.
    #[test]
    fn stale_results_are_dropped() {
        let ai = |n| ObjectIdentifier::new(ObjectType::ANALOG_INPUT, n).unwrap();
        let mut browser = Browser::new(7, "10.0.0.7:47808".into(), 1);
        browser.object_req = Some(3);
        browser.view = ViewState::Reading(ai(2));
        assert!(!browser.apply(2, view(1)), "an earlier object read");
        assert_eq!(browser.view, ViewState::Reading(ai(2)));
        assert!(!browser.apply(9, BrowseResult::Failed("x".into())));
        assert!(browser.apply(3, view(2)));
        assert!(matches!(browser.view, ViewState::Ready(_)));
        assert!(!browser.apply(3, view(2)), "answered once");
        assert!(browser.apply(
            1,
            BrowseResult::Progress {
                done: 25,
                total: 500
            }
        ));
        assert_eq!(
            browser.list,
            ListState::Reading {
                done: 25,
                total: 500
            }
        );
    }

    /// A device read started while an object read waits replaces it, and
    /// that one never answers: the properties pane must not keep saying
    /// it is reading.
    #[test]
    fn a_device_refresh_releases_a_waiting_object_read() {
        use crate::tui::app::{update, Action};
        let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let mut app = crate::tui::tests::connected_app();
        let mut browser = Browser::new(7, "10.0.0.7:47808".into(), 1);
        browser.objects.set(vec![ai]);
        app.browser = Some(Box::new(browser));
        app.screen = Screen::Browse;
        app.settle();
        let read = update(&mut app, Action::Open);
        let [Command::Browse {
            req: object_req, ..
        }] = read.as_slice()
        else {
            panic!("expected one object read, got {read:?}");
        };
        let browser = app.browser.as_deref_mut().unwrap();
        assert_eq!(browser.view, ViewState::Reading(ai));
        browser.focus = Pane::Objects;
        let refresh = update(&mut app, Action::Refresh);
        assert!(matches!(
            refresh.as_slice(),
            [Command::Browse {
                read: BrowseRead::Device,
                ..
            }]
        ));
        let browser = app.browser.as_deref_mut().unwrap();
        assert_eq!(browser.view, ViewState::Empty);
        assert!(!browser.apply(*object_req, view(1)), "the replaced read");
    }
}
