//! Object and device names for the browser's object list and the device
//! table (#1896).
//!
//! The [`NameCache`] holds the names the session has learned, keyed by
//! device (instance and address, so the same instance on two networks
//! keeps two names) and object (a device's own name under its Device
//! object). It keeps at most [`MAX_NAMES`], forgetting the least recently
//! used: a row on screen is used every tick (#1931). Names that arrive
//! free fill it first: the device card, an object read in the browser, a
//! watch row's first read and every I-Have. For the rest, each tick asks
//! for the rows the last frame showed without a name, and only those: the
//! views mark which rows those were, and only when they had room for a
//! name. One command carries every device's ask, so a tick never crowds
//! the worker's queues.
//!
//! What each device was last asked for, less what has arrived since, is
//! kept, so a tick that shows the same rows (in any order) sends nothing;
//! when the rows change, the new set replaces the old one in the worker,
//! which drops rows scrolled off before their turn. A name the device
//! refused is kept as [`Name::Unavailable`] and not asked for again until
//! the device is read afresh (its card arrives). A name it didn't answer
//! for is [`Name::Unreachable`] until the device's next I-Am.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use bacnet_ops::format::clean_name;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::objects::ListRow;
use super::watch::Screen;
use super::{App, Link};
use crate::tui::message::Command;
use crate::tui::point::{NameAsk, ReadName, ReqId};

/// Names the session keeps, the least recently used forgotten first.
pub(crate) const MAX_NAMES: usize = 4_096;

/// What the session knows of one object's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Name {
    /// Its name.
    Known(String),
    /// The device refused it.
    Unavailable,
    /// The device didn't answer; asked again after its next I-Am.
    Unreachable,
}

/// One device's name for one object.
#[derive(Debug)]
struct Entry {
    /// The device's address.
    address: String,
    name: Name,
    /// When it was last used, the key in [`NameCache::by_use`].
    used: u64,
}

/// What one device was last asked for.
#[derive(Debug, Default)]
struct Ask {
    /// The request that carried it.
    req: ReqId,
    /// The device's address it was asked under.
    address: String,
    /// The objects asked for that haven't arrived.
    objects: HashSet<ObjectIdentifier>,
    /// It reached the worker, as far as the UI knows; a refusal clears it.
    sent: bool,
    /// The next ask must tell the worker to read unreachable names again.
    retry: bool,
    /// The ask that went carried `retry`, to restore if it is refused.
    retried: bool,
}

/// The session's names, and what each device was last asked for.
#[derive(Debug, Default)]
pub(crate) struct NameCache {
    /// By instance and object; one entry per address, nearly always one.
    names: HashMap<(u32, ObjectIdentifier), Vec<Entry>>,
    /// Every entry by when it was last used, oldest first.
    by_use: BTreeMap<u64, (u32, ObjectIdentifier)>,
    clock: u64,
    /// Per instance, how many of its entries are [`Name::Unreachable`].
    stalled: HashMap<u32, usize>,
    /// Counts changes to known names, so the object filter knows to look
    /// again (#1929).
    changes: u64,
    /// Counts changes to known Device object names only, so the device
    /// table doesn't rebuild for every object name that arrives.
    device_changes: u64,
    asked: BTreeMap<u32, Ask>,
}

impl NameCache {
    /// What is known of `object`'s name on the device `device` at
    /// `address`.
    pub(crate) fn get(
        &self,
        device: u32,
        address: &str,
        object: ObjectIdentifier,
    ) -> Option<&Name> {
        self.names
            .get(&(device, object))?
            .iter()
            .find(|entry| entry.address == address)
            .map(|entry| &entry.name)
    }

    /// `object`'s name on that device, when it is known.
    pub(crate) fn known(
        &self,
        device: u32,
        address: &str,
        object: ObjectIdentifier,
    ) -> Option<&str> {
        match self.get(device, address, object)? {
            Name::Known(name) => Some(name),
            Name::Unavailable | Name::Unreachable => None,
        }
    }

    /// Bumped whenever a known name is learned, changed or forgotten.
    pub(crate) fn changes(&self) -> u64 {
        self.changes
    }

    /// Bumped whenever a device's own name (its Device object's) is
    /// learned, changed or forgotten.
    pub(crate) fn device_changes(&self) -> u64 {
        self.device_changes
    }

    /// Count a change to a known name of `object`.
    fn changed(&mut self, object: ObjectIdentifier) {
        self.changes += 1;
        if object.object_type() == ObjectType::DEVICE {
            self.device_changes += 1;
        }
    }

    /// Names held.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.by_use.len()
    }

    /// Learn a name that came free.
    pub(crate) fn learn_name(
        &mut self,
        device: u32,
        address: &str,
        object: ObjectIdentifier,
        name: String,
    ) {
        self.learn(device, address, object, ReadName::Named(name));
    }

    /// Learn how `object`'s name read went. A name once known isn't lost to
    /// a later failure, nor a refusal to a timeout. Names are remote text,
    /// so they are cleaned first ([`clean_name`]).
    pub(crate) fn learn(
        &mut self,
        device: u32,
        address: &str,
        object: ObjectIdentifier,
        read: ReadName,
    ) {
        if let Some(ask) = self.asked.get_mut(&device).filter(|a| a.address == address) {
            ask.objects.remove(&object);
        }
        let known = self.get(device, address, object);
        let name = match read {
            ReadName::Named(name) => Name::Known(clean_name(&name)),
            ReadName::Refused if !matches!(known, Some(Name::Known(_))) => Name::Unavailable,
            ReadName::Unreachable if known.is_none() => Name::Unreachable,
            ReadName::Refused | ReadName::Unreachable => return,
        };
        self.put(device, address, object, name);
    }

    /// Mark the name used now, so it is forgotten last.
    fn touch(&mut self, device: u32, address: &str, object: ObjectIdentifier) {
        self.clock += 1;
        let now = self.clock;
        let Some(entry) = self
            .names
            .get_mut(&(device, object))
            .and_then(|entries| entries.iter_mut().find(|e| e.address == address))
        else {
            return;
        };
        self.by_use.remove(&entry.used);
        entry.used = now;
        self.by_use.insert(now, (device, object));
    }

    fn put(&mut self, device: u32, address: &str, object: ObjectIdentifier, name: Name) {
        self.clock += 1;
        let now = self.clock;
        let unreachable = name == Name::Unreachable;
        let entries = self.names.entry((device, object)).or_default();
        let old = match entries.iter_mut().find(|e| e.address == address) {
            Some(entry) => {
                self.by_use.remove(&entry.used);
                entry.used = now;
                Some(std::mem::replace(&mut entry.name, name.clone()))
            }
            None => {
                entries.push(Entry {
                    address: address.to_string(),
                    name: name.clone(),
                    used: now,
                });
                None
            }
        };
        self.by_use.insert(now, (device, object));
        let was_unreachable = old.as_ref() == Some(&Name::Unreachable);
        if unreachable && !was_unreachable {
            *self.stalled.entry(device).or_default() += 1;
        } else if was_unreachable && !unreachable {
            self.unstall(device);
        }
        if matches!(name, Name::Known(_)) || matches!(old, Some(Name::Known(_))) {
            self.changed(object);
        }
        while self.by_use.len() > MAX_NAMES {
            self.evict_oldest();
        }
    }

    fn unstall(&mut self, device: u32) {
        if let Some(count) = self.stalled.get_mut(&device) {
            *count -= 1;
            if *count == 0 {
                self.stalled.remove(&device);
            }
        }
    }

    /// Forget the least recently used name.
    fn evict_oldest(&mut self) {
        let Some((used, key)) = self.by_use.pop_first() else {
            return;
        };
        let Some(entries) = self.names.get_mut(&key) else {
            return;
        };
        let entry = entries
            .iter()
            .position(|e| e.used == used)
            .map(|at| entries.swap_remove(at));
        if entries.is_empty() {
            self.names.remove(&key);
        }
        if let Some(entry) = entry {
            self.forgotten(key, &entry.name);
        }
    }

    /// Account for a forgotten entry of `key` holding `name`.
    fn forgotten(&mut self, (device, object): (u32, ObjectIdentifier), name: &Name) {
        match name {
            Name::Unreachable => self.unstall(device),
            Name::Known(_) => self.changed(object),
            Name::Unavailable => {}
        }
    }

    /// Forget the entries of `device` at `address` that `drop` picks.
    fn forget_where(&mut self, device: u32, address: &str, drop: impl Fn(&Name) -> bool) -> bool {
        let keys: Vec<(u32, ObjectIdentifier)> = self
            .names
            .keys()
            .filter(|(of, _)| *of == device)
            .copied()
            .collect();
        let mut any = false;
        for key in keys {
            let Some(entries) = self.names.get_mut(&key) else {
                continue;
            };
            let Some(at) = entries
                .iter()
                .position(|e| e.address == address && drop(&e.name))
            else {
                continue;
            };
            let entry = entries.swap_remove(at);
            if entries.is_empty() {
                self.names.remove(&key);
            }
            self.by_use.remove(&entry.used);
            self.forgotten(key, &entry.name);

            any = true;
        }
        any
    }

    /// `device` at `address` sent an I-Am: the names it didn't answer for
    /// are asked for again, and the worker told to read them.
    pub(crate) fn answered(&mut self, device: u32, address: &str) {
        if !self.stalled.contains_key(&device) {
            return;
        }
        if self.forget_where(device, address, |name| *name == Name::Unreachable) {
            self.asked.entry(device).or_default().retry = true;
        }
    }

    /// `device` at `address` was read afresh (its card arrived): every
    /// name it refused or didn't answer for is asked for again. The worker
    /// forgot them when it took the read.
    pub(crate) fn forget_failures(&mut self, device: u32, address: &str) {
        self.forget_where(device, address, |name| !matches!(name, Name::Known(_)));
    }

    /// Request `req` didn't reach the worker: what it asked, withdrawals
    /// included, is asked again on the next tick. Returns whether it was a
    /// name request.
    pub(crate) fn refused(&mut self, req: ReqId) -> bool {
        let mut found = false;
        for ask in self
            .asked
            .values_mut()
            .filter(|ask| ask.req == req && ask.sent)
        {
            ask.sent = false;
            ask.retry |= ask.retried;
            found = true;
        }
        found
    }

    /// Devices with an ask on record.
    #[cfg(test)]
    pub(crate) fn asked_devices(&self) -> usize {
        self.asked.len()
    }
}

/// The names of one device's objects, for the browser's object filter.
#[derive(Clone, Copy)]
pub(crate) struct NameLookup<'a> {
    cache: &'a NameCache,
    device: u32,
    address: &'a str,
}

impl<'a> NameLookup<'a> {
    /// The names `cache` holds for `device` at `address`.
    pub(crate) fn new(cache: &'a NameCache, device: u32, address: &'a str) -> Self {
        Self {
            cache,
            device,
            address,
        }
    }

    /// `object`'s name, when it is known.
    pub(crate) fn name(&self, object: ObjectIdentifier) -> Option<&'a str> {
        self.cache.known(self.device, self.address, object)
    }

    /// See [`NameCache::changes`].
    pub(crate) fn changes(&self) -> u64 {
        self.cache.changes()
    }
}

/// The Device object of `instance`.
pub(crate) fn device_object(instance: u32) -> Option<ObjectIdentifier> {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).ok()
}

impl App {
    /// True while names can be asked for: connected, and not passive.
    pub(crate) fn names_live(&self) -> bool {
        matches!(self.link, Link::Up { .. }) && !self.safety.passive
    }

    /// The address the device table shows for `instance`, or nothing for
    /// a device not in it.
    pub(crate) fn device_address(&self, instance: u32) -> &str {
        self.devices
            .get(instance)
            .map_or("", |row| row.address.as_str())
    }

    /// What is known of `object`'s name on `device`, at the address the
    /// device table shows for it.
    pub(crate) fn name_of(&self, device: u32, object: ObjectIdentifier) -> Option<&Name> {
        self.names.get(device, self.device_address(device), object)
    }

    /// The rows the last frame showed and had room to name.
    fn rows_on_screen(&self) -> Vec<(u32, ObjectIdentifier)> {
        let mut rows = Vec::new();
        let devices = &self.devices;
        if self.screen == Screen::Main && devices.named.get() {
            let shown = devices.visible_window(devices.offset.get(), devices.page.get());
            rows.extend(
                shown.filter_map(|row| device_object(row.instance).map(|o| (row.instance, o))),
            );
        }
        if let Some(browser) = self.shown_browser().filter(|b| b.objects.named.get()) {
            let objects = &browser.objects;
            let shown = objects.rows().iter().skip(objects.offset.get());
            for row in shown.take(objects.page.get()) {
                if let ListRow::Object(object) = row {
                    rows.push((browser.device, *object));
                }
            }
        }
        rows
    }

    /// Each tick: one command asking each device for the names on screen
    /// it hasn't been asked for, and withdrawing the ones that left it.
    /// The names on screen are marked used, and asks with nothing left to
    /// say are dropped.
    pub(super) fn ask_names(&mut self, out: &mut Vec<Command>) {
        let rows = if self.names_live() {
            self.rows_on_screen()
        } else {
            Vec::new()
        };
        let devices = &self.devices;
        let names = &mut self.names;
        names
            .asked
            .retain(|_, ask| !(ask.sent && ask.objects.is_empty() && !ask.retry));
        let mut wanted: BTreeMap<u32, Vec<ObjectIdentifier>> = BTreeMap::new();
        for (device, object) in rows {
            let address = devices.get(device).map_or("", |row| row.address.as_str());
            if names.get(device, address, object).is_some() {
                names.touch(device, address, object);
            } else {
                wanted.entry(device).or_default().push(object);
            }
        }
        let all: BTreeSet<u32> = names.asked.keys().chain(wanted.keys()).copied().collect();
        let mut asks = Vec::new();
        for device in all {
            let now = wanted.get(&device).map_or(&[][..], Vec::as_slice);
            let row = devices.get(device);
            let address = row.map_or("", |row| row.address.as_str());
            let unchanged = match names.asked.get(&device) {
                Some(ask) => {
                    ask.sent
                        && !ask.retry
                        && ask.address == address
                        && ask.objects.len() == now.len()
                        && now.iter().all(|object| ask.objects.contains(object))
                }
                None => now.is_empty(),
            };
            if !unchanged {
                let retry = names.asked.get(&device).is_some_and(|ask| ask.retry);
                asks.push(NameAsk {
                    device,
                    address: address.to_string(),
                    max_apdu: row.map(|row| row.max_apdu),
                    objects: now.to_vec(),
                    retry,
                });
            }
        }
        if asks.is_empty() {
            return;
        }
        let req = self.next_req();
        for ask in &asks {
            let entry = Ask {
                req,
                address: ask.address.clone(),
                objects: ask.objects.iter().copied().collect(),
                sent: true,
                retry: false,
                retried: ask.retry,
            };
            self.names.asked.insert(ask.device, entry);
        }
        out.push(Command::Names { req, asks });
    }
}
