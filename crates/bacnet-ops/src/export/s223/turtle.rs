//! An [`Inventory`] as Turtle text.
//!
//! Each exported object becomes one 223 property for its Present_Value,
//! with a BACnet external reference node beside it:
//!
//! - the class says whether the point is observed (an input), actuated
//!   (a commandable output or value: [`Access::Commandable`]) or neither (a
//!   value that isn't commandable), and whether it is quantifiable (an
//!   analog object or an Integer Value) or enumerated (binary and
//!   multi-state);
//! - a quantifiable point carries its QUDT unit and quantity kind when the
//!   unit table maps its Units; otherwise it gets QUDT's unknown quantity
//!   kind, which the draft requires of every quantifiable property, no
//!   unit, and a comment naming the BACnet unit. An Integer Value maps as
//!   an analog value does: it is never an input, so it is actuatable when
//!   commandable and plainly quantifiable otherwise;
//! - a binary point's kind is the ontology's neutral logical one; a
//!   multi-state point gets a kind of its own, minted from its states,
//!   with one member a state, named by its State_Text;
//! - the reference names the device and object by identifier and by name,
//!   and the property, and the command priority when the caller gave one.
//!
//! An entry that can't be written is skipped with a comment where it was,
//! saying which case it is, in text of ours only: a type that isn't one of
//! the [`EXPORTED_TYPES`], a read that failed, a property every object of
//! the type has that couldn't be read, or a point whose kind isn't its
//! type's family (a list built by hand can hold one, a device read can't).
//!
//! Node names come from identifiers only (`analog-input-3.present-value`),
//! never from device text, so they stay the same across runs and renames.
//! Device text goes only into string literals, escaped so the file is
//! ASCII; comments hold only text of ours.
//!
//! The draft's identifier pattern refuses instance 0, so an object or a
//! device numbered 0 loses that identifier string; its name still finds
//! it, and a comment marks the gap.

use std::cell::Cell;
use std::fmt::{self, Write};

use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;

use super::{
    Access, EXPORTED_TYPES, Entry, Inventory, MultiState, ONTOLOGY_SOURCE, ONTOLOGY_VERSION, Point,
    PointKind, TurtleOptions,
};
use crate::format::{is_binary, is_multistate};
use crate::label::{kebab, object_label};
use bacnet_types::qudt::qudt_unit;

/// States minted for one multi-state point at most; a device claiming more
/// gets the first this many.
const MAX_MINTED_STATES: u32 = 1024;

/// The prefixes every export declares, the device's own `d:` after them.
pub(super) const PREFIXES: [(&str, &str); 7] = [
    ("bacnet", "http://data.ashrae.org/bacnet/2020#"),
    ("qudt", "http://qudt.org/schema/qudt/"),
    ("quantitykind", "http://qudt.org/vocab/quantitykind/"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("s223", "http://data.ashrae.org/standard223#"),
    ("unit", "http://qudt.org/vocab/unit/"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
];

/// What an export wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Objects written as 223 properties.
    pub exported: usize,
    /// Objects left out: their type isn't mapped, their read failed, a
    /// property their type requires couldn't be read, or their kind isn't
    /// their type's.
    pub skipped: usize,
    /// Analog and Integer Value points whose unit has no QUDT mapping.
    pub unmapped_units: usize,
    /// Identifier strings left out because the instance is 0.
    pub instance_zero: usize,
    /// NUL characters in device text, written as U+FFFD: a string literal
    /// can't hold U+0000.
    pub replaced: usize,
}

/// `inventory` as Turtle, the same text [`write_turtle`] writes.
pub fn to_turtle(inventory: &Inventory, options: &TurtleOptions) -> String {
    let mut out = String::new();
    // Writing to a String can't fail.
    let _ = write_turtle(inventory, options, &mut out);
    out
}

/// Write `inventory` as Turtle to `out`, and say what was written.
pub fn write_turtle(
    inventory: &Inventory,
    options: &TurtleOptions,
    out: &mut impl Write,
) -> Result<Counts, fmt::Error> {
    let mut counts = count(inventory);
    let device_zero = inventory.device.instance == 0;
    let lit = Literals::default();
    // The body first, so the header can count what it replaced.
    let mut body = String::new();
    for entry in &inventory.entries {
        let (object, class) = match exported(entry) {
            Ok(exported) => exported,
            Err(why) => {
                let label = object_label(entry.object());
                writeln!(body, "# {label}: skipped, {why}.\n")?;
                continue;
            }
        };
        point(
            &lit,
            inventory,
            object,
            class,
            options,
            device_zero,
            &mut body,
        )?;
    }
    counts.replaced = lit.replaced.get();
    header(inventory, options, counts, out)?;
    out.write_str(&body)?;
    Ok(counts)
}

/// Writes string literals, counting the characters it had to replace.
#[derive(Default)]
struct Literals {
    replaced: Cell<usize>,
}

impl Literals {
    fn text(&self, text: &str) -> String {
        let (literal, replaced) = escape(text);
        self.replaced.set(self.replaced.get() + replaced);
        literal
    }
}

/// The counts the header gives, before any point is written.
fn count(inventory: &Inventory) -> Counts {
    let mut counts = Counts::default();
    for entry in &inventory.entries {
        let Ok((object, _)) = exported(entry) else {
            counts.skipped += 1;
            continue;
        };
        counts.exported += 1;
        if quantity(object).is_some_and(|units| qudt_unit(units).is_none()) {
            counts.unmapped_units += 1;
        }
        counts.instance_zero += usize::from(object.object.instance_number() == 0)
            + usize::from(inventory.device.instance == 0);
    }
    counts
}

fn header(
    inventory: &Inventory,
    options: &TurtleOptions,
    counts: Counts,
    out: &mut impl Write,
) -> fmt::Result {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    writeln!(
        out,
        "# EXPERIMENTAL: device {} as a proposed ASHRAE Standard 223 point list, written by",
        inventory.device.instance
    )?;
    writeln!(
        out,
        "# rusty-bacnet. The 223 draft and this mapping can change between releases."
    )?;
    writeln!(
        out,
        "# A point list only: it holds no equipment, connections or sensors."
    )?;
    writeln!(
        out,
        "# Mapping written against the Open223 ontology {ONTOLOGY_VERSION} from"
    )?;
    writeln!(
        out,
        "# {ONTOLOGY_SOURCE}; rusty-bacnet's golden exports were validated against it."
    )?;
    writeln!(
        out,
        "# {} exported, {} skipped, {} with an unmapped unit, {} left out for instance 0.",
        plural(counts.exported, "object", "objects"),
        counts.skipped,
        counts.unmapped_units,
        plural(counts.instance_zero, "identifier", "identifiers"),
    )?;
    if counts.replaced > 0 {
        writeln!(
            out,
            "# {} in device text written as U+FFFD.",
            plural(counts.replaced, "NUL character", "NUL characters"),
        )?;
    }
    if inventory.entries.is_empty() && inventory.stopped.is_none() {
        writeln!(
            out,
            "# The device has no object of the selected types: the model is empty."
        )?;
    }
    if let Some(stopped) = &inventory.stopped {
        writeln!(
            out,
            "# Incomplete: the reading stopped early ({}).",
            comment(stopped)
        )?;
    }
    writeln!(out)?;
    for (prefix, iri) in PREFIXES {
        writeln!(out, "@prefix {prefix}: <{iri}> .")?;
    }
    writeln!(
        out,
        "@prefix d: <{}device-{}/> .\n",
        options.base_iri(),
        inventory.device.instance
    )
}

/// What kind of 223 property a point is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    QuantifiableObservable,
    QuantifiableActuatable,
    Quantifiable,
    EnumeratedObservable,
    EnumeratedActuatable,
    Enumerable,
}

/// The point an entry writes and its class, or why the entry is skipped,
/// in text of ours: never device text, and the same for every device.
fn exported(entry: &Entry) -> Result<(&Point, Class), String> {
    if !EXPORTED_TYPES.contains(&entry.object().object_type()) {
        return Err("its type isn't mapped".into());
    }
    match entry {
        Entry::Point(point) => match Class::of(point) {
            Some(class) => Ok((point, class)),
            None => Err("its kind doesn't fit its type".into()),
        },
        Entry::Failed {
            property: Some(property),
            ..
        } => Err(format!(
            "no readable {}, which its type requires",
            kebab(property)
        )),
        Entry::Failed { property: None, .. } => Err("its read failed".into()),
    }
}

impl Class {
    /// The class of a point of one of the [`EXPORTED_TYPES`]; `None` when
    /// its kind isn't its type's family.
    fn of(object: &Point) -> Option<Self> {
        let object_type = object.object.object_type();
        let input = [
            ObjectType::ANALOG_INPUT,
            ObjectType::BINARY_INPUT,
            ObjectType::MULTI_STATE_INPUT,
        ]
        .contains(&object_type);
        let analog_type = [
            ObjectType::ANALOG_INPUT,
            ObjectType::ANALOG_OUTPUT,
            ObjectType::ANALOG_VALUE,
        ]
        .contains(&object_type);
        let (fits, analog, commanded) = match &object.kind {
            PointKind::Analog(kind) => (analog_type, true, commandable(&kind.access)),
            PointKind::Integer(kind) => (
                object_type == ObjectType::INTEGER_VALUE,
                true,
                commandable(&kind.access),
            ),
            PointKind::Binary(kind) => (is_binary(object_type), false, commandable(&kind.access)),
            PointKind::MultiState(kind) => {
                (is_multistate(object_type), false, commandable(&kind.access))
            }
            _ => (false, false, false),
        };
        if !fits {
            return None;
        }
        Some(match (analog, input, commanded) {
            (true, true, _) => Self::QuantifiableObservable,
            (true, false, true) => Self::QuantifiableActuatable,
            (true, false, false) => Self::Quantifiable,
            (false, true, _) => Self::EnumeratedObservable,
            (false, false, true) => Self::EnumeratedActuatable,
            (false, false, false) => Self::Enumerable,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::QuantifiableObservable => "s223:QuantifiableObservableProperty",
            Self::QuantifiableActuatable => "s223:QuantifiableActuatableProperty",
            Self::Quantifiable => "s223:QuantifiableProperty",
            Self::EnumeratedObservable => "s223:EnumeratedObservableProperty",
            Self::EnumeratedActuatable => "s223:EnumeratedActuatableProperty",
            Self::Enumerable => "s223:EnumerableProperty",
        }
    }

    fn actuatable(self) -> bool {
        matches!(
            self,
            Self::QuantifiableActuatable | Self::EnumeratedActuatable
        )
    }
}

/// Whether peers command Present_Value through a Priority_Array.
fn commandable<T>(access: &Access<T>) -> bool {
    matches!(access, Access::Commandable { .. })
}

/// The Units of a quantifiable point: an analog object's or an Integer
/// Value's.
fn quantity(point: &Point) -> Option<EngineeringUnits> {
    match &point.kind {
        PointKind::Analog(analog) => Some(analog.units),
        PointKind::Integer(integer) => Some(integer.units),
        _ => None,
    }
}

/// `analog-input-3`: an object's part of its node names.
fn local(object: ObjectIdentifier) -> String {
    format!(
        "{}-{}",
        kebab(object.object_type()),
        object.instance_number()
    )
}

/// `analog-input,3`: the reference's identifier string.
pub(super) fn identifier(object_type: ObjectType, instance: u32) -> String {
    format!("{},{instance}", kebab(object_type))
}

fn point(
    lit: &Literals,
    inventory: &Inventory,
    object: &Point,
    class: Class,
    options: &TurtleOptions,
    device_zero: bool,
    out: &mut impl Write,
) -> fmt::Result {
    let node = format!("d:{}.present-value", local(object.object));
    let label = &object.name;
    writeln!(out, "# {}", object_label(object.object))?;
    if let PointKind::MultiState(multi_state) = &object.kind {
        states(lit, object.object, multi_state, label, out)?;
    }
    writeln!(out, "{node} a {} ;", class.name())?;
    writeln!(out, "    rdfs:label {} ;", lit.text(label))?;
    if let Some(description) = object.description.as_deref().filter(|d| !d.is_empty()) {
        writeln!(out, "    rdfs:comment {} ;", lit.text(description))?;
    }
    match (quantity(object), &object.kind) {
        (Some(quantity), _) => units(quantity, out)?,
        (None, PointKind::MultiState(_)) => writeln!(
            out,
            "    s223:hasEnumerationKind d:{}.states ;",
            local(object.object)
        )?,
        _ => writeln!(out, "    s223:hasEnumerationKind s223:Binary-Logical ;")?,
    }
    writeln!(out, "    s223:hasExternalReference {node}.ref .\n")?;

    writeln!(out, "{node}.ref a s223:BACnetExternalReference ;")?;
    if device_zero {
        writeln!(
            out,
            "    # device 0: no device-identifier, which the draft's pattern refuses"
        )?;
    } else {
        let device = identifier(ObjectType::DEVICE, inventory.device.instance);
        writeln!(out, "    bacnet:device-identifier {} ;", lit.text(&device))?;
    }
    if let Some(name) = &inventory.device.name {
        writeln!(out, "    bacnet:device-name {} ;", lit.text(name))?;
    }
    let instance = object.object.instance_number();
    if instance == 0 {
        writeln!(
            out,
            "    # instance 0: no object-identifier, which the draft's pattern refuses"
        )?;
    } else {
        let id = identifier(object.object.object_type(), instance);
        writeln!(out, "    bacnet:object-identifier {} ;", lit.text(&id))?;
    }
    writeln!(out, "    bacnet:object-name {} ;", lit.text(label))?;
    match options.write_priority().filter(|_| class.actuatable()) {
        Some(priority) => {
            writeln!(out, "    bacnet:property-identifier \"present-value\" ;")?;
            writeln!(
                out,
                "    bacnet:priority-for-writing \"{}\"^^xsd:nonNegativeInteger .\n",
                priority.get()
            )
        }
        None => writeln!(out, "    bacnet:property-identifier \"present-value\" .\n"),
    }
}

/// A quantifiable point's QUDT terms. A unit the table doesn't map gets the
/// unknown quantity kind and no unit, since the draft wants a quantity kind
/// on every quantifiable property, and a comment naming the BACnet unit.
fn units(units: EngineeringUnits, out: &mut impl Write) -> fmt::Result {
    if let Some((unit, kind)) = qudt_unit(units) {
        writeln!(out, "    qudt:hasUnit unit:{unit} ;")?;
        return writeln!(out, "    qudt:hasQuantityKind quantitykind:{kind} ;");
    }
    writeln!(
        out,
        "    rdfs:comment \"BACnet units {}: no QUDT mapping in this export\" ;",
        kebab(units)
    )?;
    writeln!(out, "    qudt:hasQuantityKind quantitykind:Unknown ;")
}

/// A multi-state point's minted enumeration kind and its members. Each is
/// a class and an instance of itself, as the ontology's own kinds are.
fn states(
    lit: &Literals,
    object: ObjectIdentifier,
    multi_state: &MultiState,
    label: &str,
    out: &mut impl Write,
) -> fmt::Result {
    let kind = format!("d:{}.states", local(object));
    let claimed = multi_state.number_of_states;
    writeln!(out, "{kind} a s223:Class, {kind} ;")?;
    writeln!(out, "    rdfs:subClassOf s223:EnumerationKind ;")?;
    writeln!(
        out,
        "    rdfs:label {} .",
        lit.text(&format!("{label} states"))
    )?;
    if claimed > MAX_MINTED_STATES {
        writeln!(
            out,
            "# {claimed} states claimed; the first {MAX_MINTED_STATES} are listed."
        )?;
    }
    for state in 1..=claimed.min(MAX_MINTED_STATES) {
        let text = multi_state
            .state_texts
            .get(state as usize - 1)
            .filter(|text| !text.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("state {state}"));
        writeln!(out, "{kind}.{state} a s223:Class, {kind}.{state} ;")?;
        writeln!(out, "    rdfs:subClassOf {kind} ;")?;
        writeln!(out, "    rdfs:label {} .", lit.text(&text))?;
    }
    writeln!(out)
}

/// `text` as a Turtle string literal, as [`escape`] writes it.
#[cfg(test)]
pub(super) fn literal(text: &str) -> String {
    escape(text).0
}

/// `text` as a Turtle string literal: quotes, backslashes and the common
/// controls escaped by name, every other character outside printable ASCII
/// by its code point, so device text can't end the literal or the line,
/// and the file stays ASCII. U+0000, which a string literal's datatype
/// doesn't allow, becomes U+FFFD; the count of those comes back too.
fn escape(text: &str) -> (String, usize) {
    let mut out = String::with_capacity(text.len() + 2);
    let mut replaced = 0;
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\0' => {
                out.push_str("\\uFFFD");
                replaced += 1;
            }
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ' '..='~' => out.push(ch),
            _ if u32::from(ch) <= 0xFFFF => {
                let _ = write!(out, "\\u{:04X}", u32::from(ch));
            }
            _ => {
                let _ = write!(out, "\\U{:08X}", u32::from(ch));
            }
        }
    }
    out.push('"');
    (out, replaced)
}

/// Text of ours for a comment, kept to printable ASCII so nothing in it
/// can end the comment's line.
fn comment(text: &str) -> String {
    text.chars()
        .map(|ch| if (' '..='~').contains(&ch) { ch } else { '?' })
        .collect()
}
