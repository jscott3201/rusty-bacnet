//! The `/` filter line and the derived state it feeds: the device table's
//! visible order and the browser's object rows, both of which match the
//! names already in the session's cache (#1929).

use super::names::NameLookup;
use super::watch::Screen;
use super::{App, FilterKey};

impl App {
    /// Rebuild derived state (the visible table order, the object rows)
    /// before drawing.
    pub(crate) fn settle(&mut self) {
        self.settle_devices();
        self.settle_objects();
        // Only a frame on screen is decoded.
        if self.screen == Screen::Capture {
            self.capture.settle();
        }
    }

    /// Rebuild the device table's visible order, if anything changed.
    pub(super) fn settle_devices(&mut self) {
        self.devices.settle(&self.names);
    }

    /// Rebuild the open browser's object rows, if anything changed.
    pub(super) fn settle_objects(&mut self) {
        if let Some(browser) = self.browser.as_deref_mut() {
            let device = browser.device;
            let address = self
                .devices
                .get(device)
                .map_or("", |row| row.address.as_str());
            let names = NameLookup::new(&self.names, device, address);
            browser.objects.settle(&names);
        }
    }

    /// The filter of the list on screen: the browser's objects or the devices.
    pub(crate) fn filter_text(&self) -> &str {
        if self.screen == Screen::Capture {
            return &self.capture.filter;
        }
        match self.shown_browser() {
            Some(browser) => browser.objects.filter(),
            None => self.devices.filter(),
        }
    }

    pub(super) fn filter_key(&mut self, key: FilterKey) {
        if self.screen == Screen::Capture {
            return self.capture_filter_key(key);
        }
        let shown = self.screen == Screen::Browse;
        if let Some(browser) = self.browser.as_deref_mut().filter(|_| shown) {
            let objects = &mut browser.objects;
            match key {
                FilterKey::Char(ch) => objects.push_filter(ch),
                FilterKey::Backspace => objects.pop_filter(),
                FilterKey::Commit => self.filter_editing = false,
                FilterKey::Clear => {
                    objects.clear_filter();
                    self.filter_editing = false;
                }
            }
            self.settle_objects();
            return;
        }
        match key {
            FilterKey::Char(ch) => self.devices.push_filter(ch),
            FilterKey::Backspace => self.devices.pop_filter(),
            FilterKey::Commit => self.filter_editing = false,
            FilterKey::Clear => {
                self.devices.clear_filter();
                self.filter_editing = false;
            }
        }
        self.settle_devices();
    }
}
