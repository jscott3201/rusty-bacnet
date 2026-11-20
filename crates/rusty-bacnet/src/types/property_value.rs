use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use bacnet_encoding::tags::MAX_CONTEXT_NESTING_DEPTH;
#[path = "property_value_pickle.rs"]
mod pickle;

use super::constructed_read::{Element, TypedElement};
use super::date::{date_from_value, date_value};
use super::network_port_value::NetworkPortTable;
use super::timestamp::time_value;
use super::*;

// ---------------------------------------------------------------------------
// PropertyValue — typed wrapper with explicit variant constructors.
// ---------------------------------------------------------------------------

/// BACnet application-layer value.
///
/// Use typed constructors to create values:
/// ```python
/// PropertyValue.real(72.5)
/// PropertyValue.unsigned(42)
/// PropertyValue.boolean(True)
/// PropertyValue.character_string("hello")
/// PropertyValue.null()
/// ```
///
/// Read results with `.value` (native Python type) and `.tag` (type name).
///
/// `copy` and `pickle` rebuild a value through the constructor its `tag`
/// names (#1500); see `__reduce__`.
#[pyclass(
    name = "PropertyValue",
    module = "rusty_bacnet",
    frozen,
    from_py_object
)]
#[derive(Clone, Debug)]
pub struct PyPropertyValue {
    /// The value as it travels: what a write of this value encodes.
    pub(crate) inner: primitives::PropertyValue,
    /// Provenance of an actual network read, independent of the value and its write encoding.
    pub(super) context: Option<PyPropertyReadContext>,
    /// How `tag` and `value` read `inner`'s application data.
    pub(crate) form: Form,
    /// The property a read took the value from (#2080), which names its
    /// enumerated value (`name`) and its bit string's bits (`value`). Not part
    /// of the value: it doesn't take part in equality, hashing or pickling.
    pub(crate) property: Option<bacnet_types::enums::PropertyIdentifier>,
}

/// How a value's application data reads: its `inner` is one
/// `ApplicationData`, or a `List` whose `ApplicationData` items read that
/// way. `tag` and `value` follow it, while `inner` keeps the octets as read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Form {
    /// As its octets: `application_data`, and any value that has none.
    Plain,
    /// A typed read of a constructed collection (#1310): one element of
    /// this production, or the whole collection of them.
    Typed(Element),
    /// A value with no typed form read as its tags (#2618; see `tag_tree`).
    Tagged,
    /// A whole Network Port BDT or FDT, retaining its exact value bytes.
    NetworkPortTable(NetworkPortTable),
}

impl Form {
    /// The Python form of application data `octets` read this way.
    fn octets_to_py(self, py: Python<'_>, octets: &[u8]) -> PyResult<Py<PyAny>> {
        match self {
            Self::Typed(element) => element.to_py(py, octets),
            Self::NetworkPortTable(table) => table.to_py(py, octets),
            Self::Tagged => super::tag_tree::nodes_to_py(py, octets),
            Self::Plain => Ok(PyBytes::new(py, octets).into_any().unbind()),
        }
    }
}

impl PartialEq for PyPropertyValue {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner && self.form == other.form
    }
}

impl PyPropertyValue {
    pub fn to_rust(&self) -> &primitives::PropertyValue {
        &self.inner
    }

    pub fn from_rust(value: primitives::PropertyValue) -> Self {
        Self {
            inner: value,
            form: Form::Plain,
            property: None,
            context: None,
        }
    }

    /// The `tag` Python reads, for Rust code.
    pub(crate) fn tag_name(&self) -> &str {
        self.tag()
    }

    /// A read value with no typed form, read as its tags (#2618): `octets`
    /// must be what [`super::tag_tree::is_tree`] accepts.
    pub(crate) fn tree(octets: Vec<u8>) -> Self {
        Self {
            form: Form::Tagged,
            ..Self::from_rust(primitives::PropertyValue::ApplicationData(octets))
        }
    }

    /// The production of a typed constructed read, if this is one.
    #[cfg(test)]
    pub(crate) fn element(&self) -> Option<Element> {
        match self.form {
            Form::Typed(element) => Some(element),
            _ => None,
        }
    }

    /// This value as read from `property`, so its enumerated value and bit
    /// string's bits come out named.
    pub(crate) fn read_from(mut self, property: bacnet_types::enums::PropertyIdentifier) -> Self {
        self.property = Some(property);
        self
    }

    /// A typed read of constructed elements; see [`Form::Typed`].
    pub(crate) fn constructed(value: primitives::PropertyValue, element: Element) -> Self {
        Self {
            form: Form::Typed(element),
            ..Self::from_rust(value)
        }
    }
}

// ---------------------------------------------------------------------------
// PropertyValue -> native Python conversion (used by PropertyValue.value getter)
// ---------------------------------------------------------------------------

fn property_value_to_py(
    py: Python<'_>,
    value: &primitives::PropertyValue,
    property: Option<bacnet_types::enums::PropertyIdentifier>,
) -> PyResult<Py<PyAny>> {
    Ok(match value {
        primitives::PropertyValue::Null => py.None(),
        primitives::PropertyValue::Boolean(b) => {
            b.into_pyobject(py)?.to_owned().into_any().unbind()
        }
        primitives::PropertyValue::Unsigned(u) => u.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::Signed(i) => i.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::Real(f) => (*f as f64).into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::Double(f) => f.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::CharacterString(s) => {
            s.into_pyobject(py)?.to_owned().into_any().unbind()
        }
        primitives::PropertyValue::Enumerated(e) => e.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::OctetString(b) => PyBytes::new(py, b).into_any().unbind(),
        primitives::PropertyValue::ObjectIdentifier(oid) => {
            Py::new(py, PyObjectIdentifier::from_rust(*oid))?.into_any()
        }
        primitives::PropertyValue::BitString { unused_bits, data } => Py::new(
            py,
            super::PyBitString::for_property(property, *unused_bits, data),
        )?
        .into_any(),
        primitives::PropertyValue::Date(d) => date_value(d).into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::Time(t) => time_value(t).into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::List(elements) => {
            let list = pyo3::types::PyList::empty(py);
            for elem in elements {
                list.append(property_value_to_py(py, elem, property)?)?;
            }
            list.into_any().unbind()
        }
        primitives::PropertyValue::ApplicationData(bytes) => {
            match priority_date_time(bytes, property) {
                // A date-time Priority_Array slot (#2098) reads as its
                // object's Present_Value does: the date, then the time.
                Some((date, time)) => {
                    let pair = pyo3::types::PyList::empty(py);
                    pair.append(date_value(&date))?;
                    pair.append(time_value(&time))?;
                    pair.into_any().unbind()
                }
                None => PyBytes::new(py, bytes).into_any().unbind(),
            }
        }
    })
}

/// The Date and Time of a Priority_Array slot holding the date-time choice
/// of BACnetPriorityValue, context tag 1 around them; `None` for any other
/// octets or property.
fn priority_date_time(
    octets: &[u8],
    property: Option<bacnet_types::enums::PropertyIdentifier>,
) -> Option<(primitives::Date, primitives::Time)> {
    if property != Some(bacnet_types::enums::PropertyIdentifier::PRIORITY_ARRAY) {
        return None;
    }
    match bacnet_encoding::primitives::decode_priority_date_time(octets, 0) {
        Ok((date_time, end)) if end == octets.len() => Some(date_time),
        _ => None,
    }
}

#[pymethods]
impl PyPropertyValue {
    // -- Typed constructors --------------------------------------------------

    #[staticmethod]
    fn null() -> Self {
        Self::from_rust(primitives::PropertyValue::Null)
    }

    #[staticmethod]
    fn boolean(value: bool) -> Self {
        Self::from_rust(primitives::PropertyValue::Boolean(value))
    }

    #[staticmethod]
    fn unsigned(value: u64) -> Self {
        Self::from_rust(primitives::PropertyValue::Unsigned(value))
    }

    #[staticmethod]
    fn signed(value: i32) -> Self {
        Self::from_rust(primitives::PropertyValue::Signed(value))
    }

    #[staticmethod]
    fn real(value: f32) -> Self {
        Self::from_rust(primitives::PropertyValue::Real(value))
    }

    #[staticmethod]
    fn double(value: f64) -> Self {
        Self::from_rust(primitives::PropertyValue::Double(value))
    }

    #[staticmethod]
    fn character_string(value: String) -> Self {
        Self::from_rust(primitives::PropertyValue::CharacterString(value))
    }

    #[staticmethod]
    fn octet_string(value: Vec<u8>) -> Self {
        Self::from_rust(primitives::PropertyValue::OctetString(value))
    }

    #[staticmethod]
    fn enumerated(value: u32) -> Self {
        Self::from_rust(primitives::PropertyValue::Enumerated(value))
    }

    #[staticmethod]
    fn object_identifier(oid: &PyObjectIdentifier) -> Self {
        Self::from_rust(primitives::PropertyValue::ObjectIdentifier(oid.to_rust()))
    }

    /// Create a Date property value, in the form `.value` reads it back.
    ///
    /// `year` is the full year, 1900..=2154, or 255 for unspecified; any
    /// other year raises ValueError (#1501).
    /// `month` is 1-12 (or 255 for unspecified).
    /// `day` is 1-31 (or 255 for unspecified).
    /// `day_of_week` is 1=Monday..7=Sunday (or 255 for unspecified).
    #[staticmethod]
    fn date(year: u16, month: u8, day: u8, day_of_week: u8) -> PyResult<Self> {
        Ok(Self::from_rust(primitives::PropertyValue::Date(
            date_from_value((year, month, day, day_of_week))?,
        )))
    }

    /// Create a Time property value.
    ///
    /// `hour` is 0-23 (or 255 for unspecified).
    /// `minute` is 0-59 (or 255 for unspecified).
    /// `second` is 0-59 (or 255 for unspecified).
    /// `hundredths` is 0-99 (or 255 for unspecified).
    #[staticmethod]
    fn time(hour: u8, minute: u8, second: u8, hundredths: u8) -> Self {
        Self::from_rust(primitives::PropertyValue::Time(primitives::Time {
            hour,
            minute,
            second,
            hundredths,
        }))
    }

    /// Create a BitString property value.
    ///
    /// `unused_bits` is the number of unused bits in the last byte (0-7).
    /// `data` is the raw bit data bytes.
    #[staticmethod]
    fn bit_string(unused_bits: u8, data: Vec<u8>) -> Self {
        Self::from_rust(primitives::PropertyValue::BitString { unused_bits, data })
    }

    /// Create a List (array) property value from a list of PropertyValue items.
    /// Items that are all elements of one typed constructed read (#1310)
    /// make that typed collection, as a whole read of them would; tagged
    /// items (#2618) stay tagged unless the list also holds application data
    /// or typed elements.
    ///
    /// Lists nest at most 32 deep, the decoder's nesting limit
    /// (`MAX_CONTEXT_NESTING_DEPTH`), and a deeper one raises ValueError
    /// (#1506). The pickle and copy rebuilds come through here too, so no
    /// value is deeper: reading, comparing, hashing and dropping one recurse
    /// once per level.
    #[staticmethod]
    fn list(items: Vec<PyPropertyValue>) -> PyResult<Self> {
        let depth = 1 + items
            .iter()
            .map(|item| list_depth(&item.inner))
            .max()
            .unwrap_or(0);
        if depth > MAX_CONTEXT_NESTING_DEPTH {
            return Err(PyValueError::new_err(format!(
                "lists nest at most {MAX_CONTEXT_NESTING_DEPTH} deep"
            )));
        }
        let data =
            |item: &Self| matches!(item.inner, primitives::PropertyValue::ApplicationData(_));
        let all = |form: Form| items.iter().all(|item| !data(item) || item.form == form);
        let form = match items.first().map(|first| first.form) {
            Some(Form::Typed(element)) if items.iter().all(data) && all(Form::Typed(element)) => {
                Form::Typed(element)
            }
            _ if items
                .iter()
                .any(|item| data(item) && item.form == Form::Tagged)
                && all(Form::Tagged) =>
            {
                Form::Tagged
            }
            _ => Form::Plain,
        };
        let list = primitives::PropertyValue::List(items.into_iter().map(|pv| pv.inner).collect());
        Ok(Self {
            form,
            ..Self::from_rust(list)
        })
    }

    /// Create an ApplicationData value from pre-encoded application-layer
    /// bytes (a context-tagged CHOICE/SEQUENCE production, e.g. a framed
    /// BACnetEventParameter). The bytes are emitted verbatim on the wire.
    #[staticmethod]
    fn application_data(bytes: Vec<u8>) -> Self {
        Self::from_rust(primitives::PropertyValue::ApplicationData(bytes))
    }

    // -- Accessors -----------------------------------------------------------

    /// The BACnet type tag (e.g. "real", "unsigned", "boolean"). One element
    /// of a typed constructed read is named by its production
    /// ("destination", "port_permission", ...); a whole one is a "list".
    /// A whole Network Port table is tagged "bdt" or "fdt".
    #[getter]
    fn tag(&self) -> &str {
        match (&self.inner, self.form) {
            (primitives::PropertyValue::ApplicationData(_), Form::Tagged) => TAGGED,
            (primitives::PropertyValue::ApplicationData(_), Form::NetworkPortTable(table)) => {
                table.tag()
            }
            (primitives::PropertyValue::ApplicationData(_), Form::Typed(element)) => element.tag(),
            (inner, _) => tag(inner),
        }
    }

    /// The value as a native Python type (float, int, str, bool, bytes, etc.).
    /// A typed constructed element is the mapping or tuple its typed write
    /// takes, and a whole typed collection a list of them.
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match (&self.inner, self.form) {
            (
                primitives::PropertyValue::ApplicationData(octets),
                form @ (Form::Typed(_) | Form::Tagged | Form::NetworkPortTable(_)),
            ) => form.octets_to_py(py, octets),
            (primitives::PropertyValue::List(items), form @ (Form::Typed(_) | Form::Tagged)) => {
                let list = pyo3::types::PyList::empty(py);
                for item in items {
                    list.append(match item {
                        primitives::PropertyValue::ApplicationData(octets) => {
                            form.octets_to_py(py, octets)?
                        }
                        item => property_value_to_py(py, item, self.property)?,
                    })?;
                }
                Ok(list.into_any().unbind())
            }
            (inner, _) => property_value_to_py(py, inner, self.property),
        }
    }

    /// The name of an enumerated value read from a property whose
    /// enumeration is known (`"HIGH_LIMIT"` for an Event_State of 3), else
    /// `None`. `value` stays the number.
    #[getter]
    fn name(&self) -> Option<&'static str> {
        match (&self.inner, self.property) {
            (primitives::PropertyValue::Enumerated(value), Some(property)) => {
                super::bit_string::resolved_enum_name(property, *value)
            }
            _ => None,
        }
    }

    /// The value's octets as a WriteProperty of it carries them (#1499): a
    /// typed constructed element's are the octets it was read from, a whole
    /// typed collection's its elements' back to back, and a primitive's its
    /// application-tagged encoding. A value that doesn't encode raises
    /// ValueError.
    #[getter]
    fn octets<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &self.inner)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(PyBytes::new(py, &encoded))
    }

    fn __repr__(&self) -> String {
        match (&self.inner, self.form) {
            (primitives::PropertyValue::ApplicationData(octets), Form::NetworkPortTable(table)) => {
                format!("PropertyValue.{}(<{} bytes>)", table.tag(), octets.len())
            }
            (primitives::PropertyValue::ApplicationData(octets), Form::Tagged) => {
                format!("PropertyValue.{TAGGED}(<{} bytes>)", octets.len())
            }
            (primitives::PropertyValue::ApplicationData(octets), Form::Typed(element)) => {
                format!("PropertyValue.{}(<{} bytes>)", element.tag(), octets.len())
            }
            (primitives::PropertyValue::List(items), Form::Typed(element)) => format!(
                "PropertyValue.list(<{} {} elements>)",
                items.len(),
                element.tag()
            ),
            (inner, _) => repr(inner),
        }
    }

    /// Equal when both carry the same value and, for a typed constructed
    /// read, the same element production. Numbers compare as numbers, so
    /// `real(0.0) == real(-0.0)` and a NaN equals nothing; a list compares
    /// item by item, and a typed element by its octets.
    fn __eq__(&self, other: &Self) -> bool {
        self == other
    }

    /// Equal values hash alike (#1506); see [`hash_value`].
    fn __hash__(&self) -> u64 {
        let mut state = DefaultHasher::new();
        hash_value(&self.inner, &mut state);
        self.form.hash(&mut state);
        state.finish()
    }

    /// A value as its tags, from its octets (#2618), as a read of a value
    /// with no typed form gives it: `value` is a list of `TagNode`, and the
    /// octets are kept as given, so a write sends them unchanged. Octets
    /// that don't read as tagged elements, each application value one this
    /// type holds, raise ValueError.
    #[staticmethod]
    fn tagged(octets: Vec<u8>) -> PyResult<Self> {
        if !super::tag_tree::is_tree(&octets) {
            return Err(PyValueError::new_err(
                "octets don't read as tagged elements",
            ));
        }
        Ok(Self::tree(octets))
    }

    /// One typed constructed element, tagged `tag`, from the octets a read
    /// gave it: what the pickles `__reduce__` makes call. Octets that aren't
    /// exactly one element of that production raise ValueError. `octets` is
    /// `bytes`, as for `BACnetTimeStamp._from_octets`.
    #[staticmethod]
    fn _typed_element(tag: &str, octets: &[u8]) -> PyResult<Self> {
        let element = Element::from_tag(tag).ok_or_else(|| {
            // The caller's tag, cut short: it can be any string.
            let mut shown: String = tag.chars().take(64).collect();
            if shown.len() < tag.len() {
                shown.push('…');
            }
            PyValueError::new_err(format!("no typed element is tagged {shown:?}"))
        })?;
        if !element.is_one(octets) {
            return Err(PyValueError::new_err(format!(
                "octets are not exactly one {} element",
                element.tag()
            )));
        }
        Ok(Self::constructed(
            primitives::PropertyValue::ApplicationData(octets.to_vec()),
            element,
        ))
    }
}

/// How many lists deep `value` nests: 0 for a scalar, 1 for a list of
/// scalars. [`PyPropertyValue::list`] keeps every value it builds within
/// [`MAX_CONTEXT_NESTING_DEPTH`], and a decoded one is a flat list at most,
/// so the walk recurses no deeper than that.
fn list_depth(value: &primitives::PropertyValue) -> usize {
    match value {
        primitives::PropertyValue::List(items) => {
            1 + items.iter().map(list_depth).max().unwrap_or(0)
        }
        _ => 0,
    }
}

/// Feeds `value` to `state` the way the derived `PartialEq` compares it, so
/// that equal values hash alike (#1506). A float's zero hashes as +0.0,
/// since `0.0 == -0.0`; every other float, a NaN included, hashes its bits.
/// A NaN equals nothing, itself included, so its hash needn't match another.
fn hash_value(value: &primitives::PropertyValue, state: &mut impl Hasher) {
    use primitives::PropertyValue as V;
    std::mem::discriminant(value).hash(state);
    match value {
        V::Null => {}
        V::Boolean(value) => value.hash(state),
        V::Unsigned(value) => value.hash(state),
        V::Signed(value) => value.hash(state),
        V::Real(value) => (if *value == 0.0 { 0.0 } else { *value })
            .to_bits()
            .hash(state),
        V::Double(value) => (if *value == 0.0 { 0.0 } else { *value })
            .to_bits()
            .hash(state),
        V::OctetString(octets) | V::ApplicationData(octets) => octets.hash(state),
        V::CharacterString(value) => value.hash(state),
        V::BitString { unused_bits, data } => (unused_bits, data).hash(state),
        V::Enumerated(value) => value.hash(state),
        V::Date(date) => date.hash(state),
        V::Time(time) => time.hash(state),
        V::ObjectIdentifier(oid) => oid.hash(state),
        V::List(items) => {
            items.len().hash(state);
            for item in items {
                hash_value(item, state);
            }
        }
    }
}

/// The `tag` of a value read as its tags (#2618).
const TAGGED: &str = "tagged";

/// The tag of a value with no typed constructed form.
fn tag(value: &primitives::PropertyValue) -> &'static str {
    match value {
        primitives::PropertyValue::Null => "null",
        primitives::PropertyValue::Boolean(_) => "boolean",
        primitives::PropertyValue::Unsigned(_) => "unsigned",
        primitives::PropertyValue::Signed(_) => "signed",
        primitives::PropertyValue::Real(_) => "real",
        primitives::PropertyValue::Double(_) => "double",
        primitives::PropertyValue::OctetString(_) => "octet_string",
        primitives::PropertyValue::CharacterString(_) => "character_string",
        primitives::PropertyValue::BitString { .. } => "bit_string",
        primitives::PropertyValue::Enumerated(_) => "enumerated",
        primitives::PropertyValue::Date(_) => "date",
        primitives::PropertyValue::Time(_) => "time",
        primitives::PropertyValue::ObjectIdentifier(_) => "object_identifier",
        primitives::PropertyValue::List(_) => "list",
        primitives::PropertyValue::ApplicationData(_) => "application_data",
    }
}

/// The repr of a value with no typed constructed form.
fn repr(value: &primitives::PropertyValue) -> String {
    match value {
        primitives::PropertyValue::Null => "PropertyValue.null()".to_string(),
        primitives::PropertyValue::Boolean(b) => format!("PropertyValue.boolean({b})"),
        primitives::PropertyValue::Unsigned(u) => format!("PropertyValue.unsigned({u})"),
        primitives::PropertyValue::Signed(i) => format!("PropertyValue.signed({i})"),
        primitives::PropertyValue::Real(f) => format!("PropertyValue.real({f})"),
        primitives::PropertyValue::Double(f) => format!("PropertyValue.double({f})"),
        primitives::PropertyValue::CharacterString(s) => {
            format!("PropertyValue.character_string({s:?})")
        }
        primitives::PropertyValue::OctetString(b) => {
            format!("PropertyValue.octet_string(<{} bytes>)", b.len())
        }
        primitives::PropertyValue::BitString { data, .. } => {
            format!("PropertyValue.bit_string(<{} bytes>)", data.len())
        }
        primitives::PropertyValue::Enumerated(e) => format!("PropertyValue.enumerated({e})"),
        primitives::PropertyValue::Date(d) => {
            let (year, month, day, _) = date_value(d);
            format!("PropertyValue.date({year}/{month}/{day})")
        }
        primitives::PropertyValue::Time(t) => {
            format!("PropertyValue.time({}:{}:{})", t.hour, t.minute, t.second)
        }
        primitives::PropertyValue::ObjectIdentifier(oid) => {
            format!(
                "PropertyValue.object_identifier({}, {})",
                oid.object_type(),
                oid.instance_number()
            )
        }
        primitives::PropertyValue::List(elements) => {
            format!("PropertyValue.list(<{} elements>)", elements.len())
        }
        primitives::PropertyValue::ApplicationData(bytes) => {
            format!("PropertyValue.application_data(<{} bytes>)", bytes.len())
        }
    }
}

#[cfg(test)]
#[path = "property_value_tests.rs"]
mod tests;
