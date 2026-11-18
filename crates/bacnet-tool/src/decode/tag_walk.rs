//! The tag walk (#2618): service parameters read once with the shared
//! [`TagReader`], as elements for the named fields and as fields for the
//! tree.
//!
//! The reader checks the framing (Clause 20.2.1): every header parses,
//! every primitive's contents end within the octets, every closing tag
//! closes the innermost open opening tag of its number (Clause 20.2.1.3.2),
//! nothing is left open, and opening tags nest at most
//! [`MAX_TAG_TREE_DEPTH`](bacnet_encoding::tags::tree::MAX_TAG_TREE_DEPTH)
//! deep. A constructed parameter's members are read
//! by a reader of their own, started just inside its opening tag, so the
//! bound counts the levels inside each parameter, as the services'
//! decoders count a value's from its own tags: a value inside a list, such
//! as a ReadPropertyMultiple-ACK's, nests as deep as its decoder takes it.
//! The walk adds the tree's bound of [`MAX_TAG_TREE_NODES`] elements.
//!
//! The first fault stops it. What was read before the fault stays, the
//! elements open there run to the end of the octets, and the innermost of
//! them holds the fault, whose leaf covers the octets the walk didn't read:
//! from the tag at fault to the end, or none, at the end, when the octets
//! end inside an element.

use std::ops::Range;

use bacnet_encoding::primitives::decode_application_value;
use bacnet_encoding::tags::tree::{MAX_TAG_TREE_NODES, TagItem, TagReader, TagTreeFault};
use bacnet_encoding::tags::{Tag, TagClass, decode_tag};
use bacnet_ops::octets::hex_upper;
use bacnet_types::primitives::PropertyValue;

use super::{APPLICATION_TAGS, Field, value_text};

/// One element of tagged octets: a primitive, or an opening tag through its
/// closing tag with what sits between them.
#[derive(Debug, Clone)]
pub(super) struct Element<'a> {
    /// The primitive's tag, or the opening tag.
    pub(super) tag: Tag,
    /// A primitive's contents; empty for a constructed element.
    contents: &'a [u8],
    /// The length of its tag header: a primitive's, or a constructed
    /// element's opening tag, which its closing tag matches.
    header: usize,
    /// Its octets, tags included, as offsets in the octets walked; an
    /// element the walk stopped inside runs to their end.
    pub(super) range: Range<usize>,
    /// A constructed element's members, in wire order.
    pub(super) children: Vec<Element<'a>>,
    /// The fault the walk stopped at directly inside it, after its members.
    stop: Option<Stop>,
    /// Whether the walk stopped inside it, there or in a member: it has no
    /// closing tag.
    stopped: bool,
}

impl<'a> Element<'a> {
    /// Its tag number.
    pub(super) fn number(&self) -> u8 {
        self.tag.number
    }

    /// Whether it is constructed: an opening tag and what follows it.
    pub(super) fn is_constructed(&self) -> bool {
        self.tag.is_opening
    }

    /// Whether the walk stopped inside it.
    pub(super) fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// The range its tags would span around `contents` octets of value:
    /// the opening tag, the contents and a closing tag of the same length.
    pub(super) fn span_around(&self, contents: usize) -> Range<usize> {
        let start = self.range.start;
        start..start + 2 * self.header + contents
    }

    /// The length of its tag header, or of its opening tag.
    pub(super) fn header(&self) -> usize {
        self.header
    }

    /// A primitive `item` that `reader` read from `base`.
    fn primitive(item: TagItem<'a>, base: usize) -> Self {
        Self {
            tag: item.tag,
            contents: item.contents,
            header: item.octets.len() - item.contents.len(),
            range: base + item.offset..base + item.end(),
            children: Vec::new(),
            stop: None,
            stopped: false,
        }
    }

    /// The constructed element opened by `opening` (read from `base`), its
    /// `members`, and how reading them ended; whether it closed.
    fn constructed(
        data: &[u8],
        opening: TagItem<'a>,
        base: usize,
        members: Vec<Self>,
        end: End,
    ) -> (Self, bool) {
        let start = base + opening.offset;
        let mut element = Self {
            tag: opening.tag,
            contents: &[],
            header: opening.octets.len(),
            range: start..data.len(),
            children: members,
            stop: None,
            stopped: true,
        };
        match end {
            End::Closed(end) => {
                element.range.end = end;
                element.stopped = false;
            }
            End::Stopped(stop) => element.stop = stop,
            End::Ended => {
                let unclosed = TagTreeFault::Unclosed {
                    number: opening.tag.number,
                };
                element.stop = Some(Stop::new(data, unclosed, start));
            }
        }
        let closed = !element.stopped;
        (element, closed)
    }
}

/// Where a walk stopped before the end of its octets.
#[derive(Debug, Clone, Copy)]
pub(super) struct Stop {
    /// Why.
    fault: TagTreeFault,
    /// Where, in the octets walked: the tag at fault, or the opening tag
    /// left open.
    offset: usize,
    /// For contents past the end: how many octets they claim, and how many
    /// remain after the header.
    short: Option<(usize, usize)>,
    /// The length of the octets walked.
    end: usize,
}

impl Stop {
    fn new(data: &[u8], fault: TagTreeFault, offset: usize) -> Self {
        let header = decode_tag(data, offset).ok();
        let short = match (fault, header) {
            (TagTreeFault::Truncated { .. }, Some((tag, after))) => {
                Some((tag.length as usize, data.len().saturating_sub(after)))
            }
            _ => None,
        };
        Self {
            fault,
            offset,
            short,
            end: data.len(),
        }
    }

    /// What stopped the walk. Contents past the end say how many octets
    /// they claim and how many remain, which hold in any frame the octets
    /// sit in; the leaf's range says where.
    pub(super) fn text(&self) -> String {
        match (self.fault, self.short) {
            (TagTreeFault::Truncated { .. }, Some((claimed, left))) => {
                format!("contents claim {claimed} octets, {left} remain")
            }
            (TagTreeFault::Truncated { .. }, None) => "contents run past the end".into(),
            (fault, _) => fault.to_string(),
        }
    }

    /// Its leaf: the octets the walk didn't read, from the tag at fault to
    /// the end, or none, at the end, for an element the octets end inside.
    fn leaf(&self, origin: Origin<'_>) -> Field {
        let start = match self.fault {
            TagTreeFault::Unclosed { .. } => self.end,
            _ => self.offset,
        };
        Field::leaf(
            format!("Tag walk stopped: {}", self.text()),
            origin.frame + start..origin.frame + self.end,
        )
    }
}

/// How reading an element's members ended.
enum End {
    /// At its closing tag, the offset just past it.
    Closed(usize),
    /// At a fault inside it: `Some` directly inside it, `None` inside one
    /// of its members, which holds it.
    Stopped(Option<Stop>),
    /// At the end of the octets, the reader's elements all closed.
    Ended,
}

/// Read with `reader` (which reads from `base`) the members of the element
/// it is inside, up to that element's closing tag, a fault or the end.
fn members<'a>(
    data: &'a [u8],
    base: usize,
    reader: &mut TagReader<'a>,
    nodes: &mut usize,
) -> (Vec<Element<'a>>, End) {
    let mut read = Vec::new();
    loop {
        let item = match reader.next() {
            Some(Ok(item)) => item,
            Some(Err(error)) => {
                let stop = Stop::new(data, error.fault, base + error.offset);
                return (read, End::Stopped(Some(stop)));
            }
            None => return (read, End::Ended),
        };
        if item.tag.is_closing {
            return (read, End::Closed(base + item.end()));
        }
        // Counted as the tree counts them: a constructed element once.
        if *nodes == MAX_TAG_TREE_NODES {
            let stop = Stop::new(data, TagTreeFault::TooManyNodes, base + item.offset);
            return (read, End::Stopped(Some(stop)));
        }
        *nodes += 1;
        if !item.tag.is_opening {
            read.push(Element::primitive(item, base));
            continue;
        }
        let (inner, end) = members(data, base, reader, nodes);
        let (element, closed) = Element::constructed(data, item, base, inner, end);
        read.push(element);
        if !closed {
            return (read, End::Stopped(None));
        }
    }
}

/// Tagged octets read whole: their elements, and where the walk stopped if
/// it stopped early.
#[derive(Debug, Clone)]
pub(super) struct Walk<'a> {
    /// The top-level elements read, in wire order.
    pub(super) elements: Vec<Element<'a>>,
    /// The fault at the top level, outside every element.
    top: Option<Stop>,
    /// The fault the walk stopped at, wherever it is.
    stop: Option<Stop>,
}

impl<'a> Walk<'a> {
    /// Walk `data` from its first octet to its first fault or its end.
    pub(super) fn new(data: &'a [u8]) -> Self {
        let mut elements = Vec::new();
        let mut nodes = 0usize;
        let mut top = None;
        let mut pos = 0;
        while pos < data.len() {
            // Each top-level tag read by a reader of its own.
            let mut reader = TagReader::new(&data[pos..]);
            let item = match reader.next() {
                Some(Ok(item)) => item,
                Some(Err(error)) => {
                    top = Some(Stop::new(data, error.fault, pos + error.offset));
                    break;
                }
                None => break,
            };
            if nodes == MAX_TAG_TREE_NODES {
                top = Some(Stop::new(data, TagTreeFault::TooManyNodes, pos));
                break;
            }
            nodes += 1;
            if !item.tag.is_opening {
                elements.push(Element::primitive(item, pos));
                pos += item.end();
                continue;
            }
            // Its members, read from just inside it: its own closing tag is
            // then one the inner reader has nothing open for.
            let base = pos + item.end();
            let mut inside = TagReader::new(&data[base..]);
            let (inner, end) = members(data, base, &mut inside, &mut nodes);
            let end = match end {
                End::Stopped(Some(mut stop)) => match stop.fault {
                    TagTreeFault::UnmatchedClosing { number, open: None }
                        if number == item.tag.number =>
                    {
                        // A closing tag is one octet, or two for a tag
                        // number from 15 up; the reader has read its header.
                        let width = if number < 15 { 1 } else { 2 };
                        End::Closed(stop.offset + width)
                    }
                    TagTreeFault::UnmatchedClosing { number, open: None } => {
                        stop.fault = TagTreeFault::UnmatchedClosing {
                            number,
                            open: Some(item.tag.number),
                        };
                        End::Stopped(Some(stop))
                    }
                    _ => End::Stopped(Some(stop)),
                },
                end => end,
            };
            let (element, closed) = Element::constructed(data, item, pos, inner, end);
            pos = element.range.end;
            elements.push(element);
            if !closed {
                break;
            }
        }
        let stop = top.or_else(|| innermost(&elements));
        Self {
            elements,
            top,
            stop,
        }
    }

    /// The fault the walk stopped at, wherever it is.
    pub(super) fn stop(&self) -> Option<&Stop> {
        self.stop.as_ref()
    }

    /// The fault at the top level, outside every element.
    pub(super) fn top_stop(&self) -> Option<&Stop> {
        self.top.as_ref()
    }

    /// The tree's fields for the walk: each element's, then the leaf of a
    /// fault at the top level.
    pub(super) fn fields(&self, origin: Origin<'_>) -> Vec<Field> {
        let mut fields = fields(&self.elements, origin);
        fields.extend(self.top.map(|stop| stop.leaf(origin)));
        fields
    }
}

/// The fault held by the last element of `elements` or one of its members.
fn innermost(elements: &[Element<'_>]) -> Option<Stop> {
    let last = elements.last().filter(|element| element.stopped)?;
    last.stop.or_else(|| innermost(&last.children))
}

/// Where walked octets sit: in the frame, as the fields' ranges count, and
/// in the octets their application values are decoded from, whose offsets
/// a value's decode error quotes.
#[derive(Debug, Clone, Copy)]
pub(super) struct Origin<'v> {
    /// Where the walked octets start in the frame.
    pub(super) frame: usize,
    /// The octets application values are decoded from.
    pub(super) values: &'v [u8],
    /// Where the walked octets start in [`Origin::values`].
    pub(super) shift: usize,
}

/// The fields of `elements`: a constructed element as a node of its
/// members, a primitive as a leaf naming its value.
pub(super) fn fields(elements: &[Element<'_>], origin: Origin<'_>) -> Vec<Field> {
    elements
        .iter()
        .map(|element| field(element, origin))
        .collect()
}

/// The fields of a constructed element's members, then the leaf of the
/// fault it holds, if any.
pub(super) fn members_of(element: &Element<'_>, origin: Origin<'_>) -> Vec<Field> {
    let mut fields = fields(&element.children, origin);
    fields.extend(element.stop.map(|stop| stop.leaf(origin)));
    fields
}

fn field(element: &Element<'_>, origin: Origin<'_>) -> Field {
    let range = origin.frame + element.range.start..origin.frame + element.range.end;
    let tag = element.tag;
    if tag.is_opening {
        let members = members_of(element, origin);
        return Field::node(format!("[{}] opening", tag.number), range, members);
    }
    if tag.class != TagClass::Application {
        let octets = element.contents;
        let number =
            (octets.len() <= 4).then(|| octets.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)));
        let label = match number {
            Some(n) if !octets.is_empty() => {
                format!("[{}] {} ({n})", tag.number, hex_upper(octets))
            }
            _ => format!("[{}] {}", tag.number, hex_upper(octets)),
        };
        return Field::leaf(label, range);
    }
    match decode_application_value(origin.values, origin.shift + element.range.start) {
        Ok((value, _)) => {
            let name = APPLICATION_TAGS
                .get(usize::from(tag.number))
                .unwrap_or(&"Application");
            let mut field = Field::leaf(format!("{name}: {}", value_text(&value)), range);
            field.device_text = matches!(value, PropertyValue::CharacterString(_));
            field
        }
        Err(error) => Field::leaf(format!("Application tag {}: {error}", tag.number), range),
    }
}

#[cfg(test)]
#[path = "tag_walk_tests.rs"]
mod tests;
