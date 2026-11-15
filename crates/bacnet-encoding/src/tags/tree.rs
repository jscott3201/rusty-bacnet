//! Tagged octets read without their datatype (#2618): any well-formed BACnet
//! value, standard or proprietary, as the tags it is made of.
//!
//! The tagging rules let a stream be parsed with no knowledge of the
//! production it encodes (Clause 20.2.1.6): an application tag names its
//! datatype (Clause 20.2.1.4), a primitive's header gives its length
//! (Clause 20.2.1.3.1), and a constructed element sits between an opening
//! and a closing tag of the same number (Clause 20.2.1.3.2). What a walk
//! can't learn is a context tag's meaning: the datatype of a context-tagged
//! primitive is the field's, known only from the production (Clauses 20.2
//! and 20.2.1.5), so its contents stay octets. The datatypes of a
//! proprietary object type's properties are built from the application
//! datatypes (Clause 23.4.2), and a vendor's property on a standard object
//! travels in the same services as a standard one (Clause 23.3), so its
//! value is tagged the same way: a well-formed proprietary value reads as
//! its tags, its meaning aside.
//!
//! Two layers:
//!
//! - [`TagReader`] yields each tag in wire order as a [`TagItem`]: its
//!   header, where it sits, how deep, and its exact octets. It needs no
//!   allocator and types nothing; [`TagItem::application_value`] types an
//!   application-tagged primitive when asked.
//! - `decode_tag_tree` (with `alloc`) builds the tree of `TagNode`s, each
//!   application primitive typed with the crate's own decoder.
//!
//! Neither alters the octets, and every item and node borrows the exact
//! octets it was read from. Both are bounded: opening tags nest at most
//! [`MAX_TAG_TREE_DEPTH`] deep, and a tree holds at most
//! [`MAX_TAG_TREE_NODES`] elements. A fault is a [`TagTreeError`] naming
//! where it is and how much of the input before it is complete
//! ([`TagTreeError::valid_up_to`]), so a caller can show the elements that
//! read and keep the rest as octets.

use core::fmt;
use core::iter::FusedIterator;

use bacnet_types::error::{DecodeError, DecodingKind, Error};

use super::{MAX_CONTEXT_NESTING_DEPTH, Tag, TagClass, app_tag, decode_tag};
use crate::primitives::{ApplicationValueRef, decode_application_value_ref};

#[cfg(feature = "alloc")]
mod node;

#[cfg(feature = "alloc")]
pub use node::{TagBody, TagNode, decode_tag_tree};

/// How deep opening tags may nest, the outermost counting as 1: the crate's
/// nesting limit, [`MAX_CONTEXT_NESTING_DEPTH`]. An opening tag past it is
/// [`TagTreeFault::TooDeep`].
pub const MAX_TAG_TREE_DEPTH: usize = MAX_CONTEXT_NESTING_DEPTH;

/// How many elements one tree may hold, a constructed one counting once: the
/// bound the crate's constructed decoders put on a decoded list (10,000), as
/// `bacnet-services` does on its sequences. Only the tree (`decode_tag_tree`,
/// with `alloc`) counts, since its size grows with them; past it is
/// [`TagTreeFault::TooManyNodes`].
pub const MAX_TAG_TREE_NODES: usize = crate::constructed::MAX_FRAMED_ITEMS;

/// One tag [`TagReader`] read, borrowed from the octets it was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TagItem<'a> {
    /// The tag header as [`decode_tag`] decoded it.
    pub tag: Tag,
    /// Where the tag header starts in the octets read.
    pub offset: usize,
    /// How many opening tags enclose it: 0 at the top level. A closing tag
    /// has the depth of the opening tag it closes.
    pub depth: usize,
    /// The item's octets exactly as read: the header and the contents of a
    /// primitive, the header alone of an opening or closing tag.
    pub octets: &'a [u8],
    /// A primitive's contents, after its header; empty for an opening or
    /// closing tag and for an application BOOLEAN, whose value is in its
    /// header (Clause 20.2.3).
    pub contents: &'a [u8],
}

impl<'a> TagItem<'a> {
    /// The offset just past the item.
    pub fn end(&self) -> usize {
        self.offset + self.octets.len()
    }

    /// Whether it is a primitive: neither an opening nor a closing tag.
    pub fn is_primitive(&self) -> bool {
        !self.tag.is_opening && !self.tag.is_closing
    }

    /// The value of an application-tagged primitive, typed by its tag
    /// number as [`decode_application_value_ref`] types it; `None` for a
    /// context tag. Contents that aren't a valid encoding of that datatype,
    /// or a tag number the standard reserves (13 and up), are the decoder's
    /// refusal, its offset in the octets read.
    pub fn application_value(&self) -> Option<Result<ApplicationValueRef<'a>, DecodeError>> {
        if self.tag.class != TagClass::Application {
            return None;
        }
        Some(
            decode_application_value_ref(self.octets, 0)
                .map(|(value, _)| value)
                .map_err(|error| {
                    let at = error.decode_fault().map_or(0, |(at, _)| at);
                    error.to_decode_error(self.offset.saturating_add(at))
                }),
        )
    }
}

/// An opening tag the reader is inside: its number and where it is.
#[derive(Debug, Clone, Copy, Default)]
struct Open {
    number: u8,
    offset: usize,
}

/// Each tag of some octets in wire order, opening and closing tags included,
/// as `Result<TagItem, TagTreeError>`.
///
/// It checks the framing only: every header parses, every primitive's
/// contents end within the octets, every closing tag closes the innermost
/// open opening tag of its number, no opening tag is left open at the end,
/// and opening tags nest at most [`MAX_TAG_TREE_DEPTH`] deep. The first
/// fault is its last item; after it, and after the end, it yields nothing.
/// Contents aren't typed ([`TagItem::application_value`] does that) and
/// items aren't counted: the work is bounded by the input's length, and
/// the reader holds no more than its fixed-size stack of open tags.
///
/// ```
/// use bacnet_encoding::tags::tree::TagReader;
///
/// // [0] 2, then [1] ( REAL 1.5 ).
/// let octets = [0x09, 0x02, 0x1E, 0x44, 0x3F, 0xC0, 0x00, 0x00, 0x1F];
/// let items: Vec<_> = TagReader::new(&octets).collect::<Result<_, _>>()?;
/// assert_eq!(items.len(), 4);
/// assert_eq!((items[0].tag.number, items[0].contents), (0, &[0x02][..]));
/// assert!(items[1].tag.is_opening && items[3].tag.is_closing);
/// assert_eq!((items[2].depth, items[2].octets.len()), (1, 5));
/// # Ok::<(), bacnet_encoding::tags::tree::TagTreeError>(())
/// ```
#[derive(Debug, Clone)]
pub struct TagReader<'a> {
    data: &'a [u8],
    /// Where the next tag starts.
    pos: usize,
    /// The open opening tags, outermost first; the first `depth` are live.
    open: [Open; MAX_TAG_TREE_DEPTH],
    depth: usize,
    /// The end of the last complete top-level element.
    valid_up_to: usize,
    done: bool,
}

impl<'a> TagReader<'a> {
    /// A reader of `data` from its first octet.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            open: [Open::default(); MAX_TAG_TREE_DEPTH],
            depth: 0,
            valid_up_to: 0,
            done: false,
        }
    }

    /// Where the next tag starts.
    pub fn offset(&self) -> usize {
        self.pos
    }

    /// How many opening tags are open where the next tag starts.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// The end of the last complete top-level element read so far.
    pub fn valid_up_to(&self) -> usize {
        self.valid_up_to
    }

    /// `fault` at `offset`, with what is complete before it.
    fn fault(&self, fault: TagTreeFault, offset: usize) -> TagTreeError {
        TagTreeError {
            fault,
            offset,
            valid_up_to: self.valid_up_to,
        }
    }

    /// The next item, `None` at a clean end.
    fn step(&mut self) -> Option<Result<TagItem<'a>, TagTreeError>> {
        let data = self.data;
        let offset = self.pos;
        if offset >= data.len() {
            let inner = self.depth.checked_sub(1)?;
            let open = self.open[inner];
            let unclosed = TagTreeFault::Unclosed {
                number: open.number,
            };
            return Some(Err(self.fault(unclosed, open.offset)));
        }
        let (tag, after) = match decode_tag(data, offset) {
            Ok(read) => read,
            Err(error) => {
                let at = error.decode_fault().map_or(offset, |(at, _)| at);
                let header = TagTreeFault::Header(error.to_decode_error(at));
                return Some(Err(self.fault(header, offset)));
            }
        };
        let depth = self.depth;
        let (end, item_depth) = if tag.is_closing {
            let open = depth.checked_sub(1).map(|inner| self.open[inner].number);
            if open != Some(tag.number) {
                let unmatched = TagTreeFault::UnmatchedClosing {
                    number: tag.number,
                    open,
                };
                return Some(Err(self.fault(unmatched, offset)));
            }
            self.depth -= 1;
            (after, self.depth)
        } else if tag.is_opening {
            if depth == MAX_TAG_TREE_DEPTH {
                return Some(Err(self.fault(TagTreeFault::TooDeep, offset)));
            }
            self.open[depth] = Open {
                number: tag.number,
                offset,
            };
            self.depth += 1;
            (after, depth)
        } else if tag.class == TagClass::Application && tag.number == app_tag::BOOLEAN {
            (after, depth)
        } else {
            match after.checked_add(tag.length as usize) {
                Some(end) if end <= data.len() => (end, depth),
                need => {
                    let truncated = TagTreeFault::Truncated {
                        need: need.unwrap_or(usize::MAX),
                        have: data.len(),
                    };
                    return Some(Err(self.fault(truncated, offset)));
                }
            }
        };
        self.pos = end;
        if self.depth == 0 {
            self.valid_up_to = end;
        }
        let primitive = !tag.is_opening && !tag.is_closing;
        Some(Ok(TagItem {
            tag,
            offset,
            depth: item_depth,
            octets: data.get(offset..end).unwrap_or_default(),
            contents: if primitive {
                data.get(after..end).unwrap_or_default()
            } else {
                &[]
            },
        }))
    }
}

impl<'a> Iterator for TagReader<'a> {
    type Item = Result<TagItem<'a>, TagTreeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let next = self.step();
        if !matches!(next, Some(Ok(_))) {
            self.done = true;
        }
        next
    }
}

impl FusedIterator for TagReader<'_> {}

/// Why a walk of tagged octets stopped before their end, where, and how much
/// of them read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TagTreeError {
    /// What is wrong.
    pub fault: TagTreeFault,
    /// Where: the first octet of the tag at fault, or of the opening tag
    /// left open for [`TagTreeFault::Unclosed`].
    pub offset: usize,
    /// The octets before this offset are whole top-level elements, so a
    /// walk of just them succeeds; the rest, from the element the fault is
    /// in, is what a caller keeps as octets.
    pub valid_up_to: usize,
}

/// What stopped a walk of tagged octets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TagTreeFault {
    /// A tag header that doesn't parse: cut short, or a form, tag number or
    /// length encoding the standard reserves or [`decode_tag`] refuses (a
    /// length that isn't in its shortest form, or past its sanity bound).
    /// The header decoder's refusal, at its own offset.
    Header(DecodeError),
    /// A primitive whose contents run past the end: they need the octets up
    /// to `need`, and there are `have`.
    Truncated {
        /// The offset the contents end at.
        need: usize,
        /// The length of the octets read.
        have: usize,
    },
    /// The octets end inside the constructed element opened by tag
    /// `number`.
    Unclosed {
        /// The opening tag's number.
        number: u8,
    },
    /// A closing tag that closes nothing open: no opening tag is open
    /// (`open` is `None`), or the innermost open one has another number.
    UnmatchedClosing {
        /// The closing tag's number.
        number: u8,
        /// The innermost open opening tag's number.
        open: Option<u8>,
    },
    /// An opening tag nested deeper than [`MAX_TAG_TREE_DEPTH`].
    TooDeep,
    /// An element past [`MAX_TAG_TREE_NODES`] in one tree; only the tree
    /// (`decode_tag_tree`, with `alloc`) counts them.
    TooManyNodes,
}

impl fmt::Display for TagTreeFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Header(refused) => write!(f, "malformed tag header: {}", refused.message()),
            Self::Truncated { need, have } => {
                write!(f, "contents end at octet {need}, past the end at {have}")
            }
            Self::Unclosed { number } => write!(f, "opening tag [{number}] is never closed"),
            Self::UnmatchedClosing { number, open: None } => {
                write!(f, "closing tag [{number}] with no opening tag")
            }
            Self::UnmatchedClosing {
                number,
                open: Some(open),
            } => write!(f, "closing tag [{number}] inside opening tag [{open}]"),
            Self::TooDeep => write!(f, "opening tags nest deeper than {MAX_TAG_TREE_DEPTH}"),
            Self::TooManyNodes => write!(f, "more than {MAX_TAG_TREE_NODES} tagged elements"),
        }
    }
}

impl fmt::Display for TagTreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tagged octets at offset {}: {}", self.offset, self.fault)
    }
}

impl core::error::Error for TagTreeError {}

impl From<TagTreeError> for Error {
    /// The fault as the crate's other decoders report it: a malformed
    /// header as the header decoder's refusal, contents past the end as
    /// [`Error::BufferTooShort`], and the rest as [`Error::Decoding`]: an
    /// open element [`DecodingKind::Missing`], an unmatched closing tag or
    /// one nested too deep [`DecodingKind::InvalidTag`], and too many
    /// elements [`DecodingKind::Overflow`].
    fn from(error: TagTreeError) -> Self {
        let offset = error.offset;
        let refused = match error.fault {
            TagTreeFault::Header(refused) => refused,
            TagTreeFault::Truncated { need, have } => return Error::buffer_too_short(need, have),
            TagTreeFault::Unclosed { number } => {
                DecodeError::new(DecodingKind::Missing, offset, "missing closing tag {}")
                    .with_value(number.into())
            }
            TagTreeFault::UnmatchedClosing { number, open: None } => DecodeError::new(
                DecodingKind::InvalidTag,
                offset,
                "closing tag {} with no opening tag",
            )
            .with_value(number.into()),
            TagTreeFault::UnmatchedClosing {
                number,
                open: Some(open),
            } => DecodeError::new(
                DecodingKind::InvalidTag,
                offset,
                "closing tag {} does not match opening tag {}",
            )
            .with_values(number.into(), open.into()),
            TagTreeFault::TooDeep => DecodeError::new(
                DecodingKind::InvalidTag,
                offset,
                "context tag nesting depth exceeds maximum ({})",
            )
            .with_value(MAX_TAG_TREE_DEPTH as u64),
            TagTreeFault::TooManyNodes => DecodeError::new(
                DecodingKind::Overflow,
                offset,
                "tagged elements exceed the limit ({})",
            )
            .with_value(MAX_TAG_TREE_NODES as u64),
        };
        refused.into()
    }
}

#[cfg(test)]
mod tests;
