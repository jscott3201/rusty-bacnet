//! The tag walk (#2618): service parameters read once with the shared
//! [`TagReader`], as elements for the named fields and as fields for the
//! tree.
//!
//! The reader checks the framing (Clause 20.2.1): every header parses,
//! every primitive's contents end within the octets, every closing tag
//! closes the innermost open opening tag of its number (Clause 20.2.1.3.2),
//! nothing is left open, and opening tags nest at most
//! [`MAX_TAG_TREE_DEPTH`](bacnet_encoding::tags::tree::MAX_TAG_TREE_DEPTH)
//! deep; the walk adds the tree's bound of [`MAX_TAG_TREE_NODES`] elements.
//! The first fault stops it. What was read before the fault stays, the
//! elements open there run to the end of the octets, and the tree marks the
//! fault with a leaf from where it is to the end, so every octet still has
//! a field.

use std::ops::Range;

use bacnet_encoding::primitives::decode_application_value;
use bacnet_encoding::tags::TagClass;
use bacnet_encoding::tags::tree::{MAX_TAG_TREE_NODES, TagItem, TagReader, TagTreeFault};
use bacnet_ops::octets::hex_upper;
use bacnet_types::primitives::PropertyValue;

use super::{APPLICATION_TAGS, Field, value_text};

/// One element of tagged octets: a primitive, or an opening tag through its
/// closing tag with what sits between them.
#[derive(Debug, Clone)]
pub(super) struct Element<'a> {
    /// The primitive, or the opening tag.
    pub(super) item: TagItem<'a>,
    /// Its octets, tags included, as offsets in the octets walked; an
    /// element the walk stopped inside runs to their end.
    pub(super) range: Range<usize>,
    /// A constructed element's members, in wire order.
    pub(super) children: Vec<Element<'a>>,
}

impl Element<'_> {
    /// Its tag number.
    pub(super) fn number(&self) -> u8 {
        self.item.tag.number
    }

    /// Whether it is constructed: an opening tag and what follows it.
    pub(super) fn is_constructed(&self) -> bool {
        self.item.tag.is_opening
    }
}

/// Where a walk stopped before the end of its octets.
#[derive(Debug, Clone, Copy)]
struct Stop {
    /// Why.
    fault: TagTreeFault,
    /// Where, in the octets walked: the tag at fault, or the opening tag
    /// left open.
    offset: usize,
    /// How many elements were open there; the fault's leaf goes in the
    /// innermost.
    depth: usize,
}

/// Tagged octets read whole: their elements, and where the walk stopped if
/// it stopped early.
#[derive(Debug, Clone)]
pub(super) struct Walk<'a> {
    /// The top-level elements read, in wire order.
    pub(super) elements: Vec<Element<'a>>,
    stop: Option<Stop>,
    /// The length of the octets walked.
    len: usize,
}

impl<'a> Walk<'a> {
    /// Walk `data` from its first octet to its first fault or its end.
    pub(super) fn new(data: &'a [u8]) -> Self {
        let mut top = Vec::new();
        // The open constructed elements, outermost first.
        let mut open: Vec<Element<'a>> = Vec::new();
        let mut nodes = 0usize;
        let mut stop = None;
        for item in TagReader::new(data) {
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    stop = Some(Stop {
                        fault: error.fault,
                        offset: error.offset,
                        depth: open.len(),
                    });
                    break;
                }
            };
            if item.tag.is_closing {
                let Some(mut element) = open.pop() else {
                    // The reader only yields a closing tag it matched.
                    stop = Some(Stop {
                        fault: TagTreeFault::UnmatchedClosing {
                            number: item.tag.number,
                            open: None,
                        },
                        offset: item.offset,
                        depth: 0,
                    });
                    break;
                };
                element.range.end = item.end();
                place(&mut open, &mut top, element);
                continue;
            }
            // Counted as the tree counts them: a constructed element once.
            nodes += 1;
            if nodes > MAX_TAG_TREE_NODES {
                stop = Some(Stop {
                    fault: TagTreeFault::TooManyNodes,
                    offset: item.offset,
                    depth: open.len(),
                });
                break;
            }
            let element = Element {
                item,
                range: item.offset..item.end(),
                children: Vec::new(),
            };
            if item.tag.is_opening {
                open.push(element);
            } else {
                place(&mut open, &mut top, element);
            }
        }
        while let Some(mut element) = open.pop() {
            element.range.end = data.len();
            place(&mut open, &mut top, element);
        }
        Self {
            elements: top,
            stop,
            len: data.len(),
        }
    }

    /// Whether the walk read every octet without a fault.
    pub(super) fn is_whole(&self) -> bool {
        self.stop.is_none()
    }

    /// The tree's fields for the walk: each element's, then a leaf for the
    /// fault, if any, inside the elements open where the walk stopped.
    pub(super) fn fields(&self, origin: Origin<'_>) -> Vec<Field> {
        let mut fields = fields(&self.elements, origin);
        let Some(stop) = self.stop else {
            return fields;
        };
        let frame = origin.frame;
        let fault = match stop.fault {
            // Its offsets as the fields' ranges count them.
            TagTreeFault::Truncated { need, have } => TagTreeFault::Truncated {
                need: need.saturating_add(frame),
                have: have + frame,
            },
            fault => fault,
        };
        let leaf = Field::leaf(
            format!("Tag walk stopped: {fault}"),
            frame + stop.offset..frame + self.len,
        );
        // The elements open at the fault are the last at each level.
        let mut level = &mut fields;
        for _ in 0..stop.depth {
            let Some(last) = level.len().checked_sub(1) else {
                break;
            };
            level = &mut level[last].children;
        }
        level.push(leaf);
        fields
    }
}

/// Add `element` to the innermost open element, or to the top level.
fn place<'a>(open: &mut [Element<'a>], top: &mut Vec<Element<'a>>, element: Element<'a>) {
    match open.last_mut() {
        Some(parent) => parent.children.push(element),
        None => top.push(element),
    }
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

fn field(element: &Element<'_>, origin: Origin<'_>) -> Field {
    let range = origin.frame + element.range.start..origin.frame + element.range.end;
    let tag = element.item.tag;
    if tag.is_opening {
        let members = fields(&element.children, origin);
        return Field::node(format!("[{}] opening", tag.number), range, members);
    }
    if tag.class != TagClass::Application {
        let octets = element.item.contents;
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
