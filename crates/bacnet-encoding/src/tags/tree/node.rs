//! The owned tree of tags, built on [`TagReader`]. Only with `alloc`.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::error::DecodeError;

use super::{MAX_TAG_TREE_NODES, TagItem, TagReader, TagTreeError, TagTreeFault};
use crate::primitives::ApplicationValueRef;
use crate::tags::TagClass;

/// One element of tagged octets, borrowed from them: a tag and what it
/// holds.
///
/// A [`TagItem`] carries the whole decoded [`Tag`](crate::tags::Tag), since
/// an opening or closing tag is an item of its own there; a node joins such
/// a pair into one element, whose length and opening or closing flags no
/// longer apply, so it keeps the class and number that name it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TagNode<'a> {
    /// The tag's class: [`TagClass::Application`] for an application
    /// primitive, [`TagClass::Context`] for a context primitive or a
    /// constructed element.
    pub class: TagClass,
    /// The tag number. An application tag's names its datatype (the
    /// numbering of Clause 20.2.1.4); a context tag's names a field.
    pub number: u8,
    /// Where the element starts in the octets read.
    pub offset: usize,
    /// The element's octets exactly as read: a primitive's header and
    /// contents, or a constructed element's opening tag through its closing
    /// tag.
    pub octets: &'a [u8],
    /// What it holds.
    pub body: TagBody<'a>,
    /// The length of its tag header: a primitive's, or a constructed
    /// element's opening tag, which its closing tag matches.
    header: u8,
}

/// What a [`TagNode`] holds.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum TagBody<'a> {
    /// An application-tagged primitive, typed by its tag number. Its owned
    /// form is `decode_application_value` of the node's octets.
    Application(ApplicationValueRef<'a>),
    /// An application-tagged primitive whose contents aren't a valid
    /// encoding of the datatype its number names, or whose number the
    /// standard reserves. Its framing is sound, so the walk goes on past it.
    Invalid {
        /// The contents, after the header.
        contents: &'a [u8],
        /// Why they don't decode, at its offset in the octets read.
        error: DecodeError,
    },
    /// A context-tagged primitive's contents. Their datatype is the
    /// field's, which only the production knows, so they stay octets.
    Context(&'a [u8]),
    /// A constructed element's members, in wire order: what sits between
    /// its opening and closing tags.
    Constructed(Vec<TagNode<'a>>),
}

impl<'a> TagNode<'a> {
    /// What the element holds as octets: a primitive's contents (none for
    /// an application BOOLEAN, whose value is in its header), or what sits
    /// between a constructed element's opening and closing tags.
    pub fn contents(&self) -> &'a [u8] {
        let header = usize::from(self.header);
        let end = if self.is_constructed() {
            self.octets.len().saturating_sub(header)
        } else {
            self.octets.len()
        };
        self.octets.get(header..end).unwrap_or_default()
    }

    /// A constructed element's members; none for a primitive.
    pub fn children(&self) -> &[TagNode<'a>] {
        match &self.body {
            TagBody::Constructed(children) => children,
            _ => &[],
        }
    }

    /// Whether it is a constructed element.
    pub fn is_constructed(&self) -> bool {
        matches!(self.body, TagBody::Constructed(_))
    }

    /// The node for a primitive `item`.
    fn primitive(item: &TagItem<'a>) -> Self {
        let body = match item.application_value() {
            Some(Ok(value)) => TagBody::Application(value),
            Some(Err(error)) => TagBody::Invalid {
                contents: item.contents,
                error,
            },
            None => TagBody::Context(item.contents),
        };
        Self {
            class: item.tag.class,
            number: item.tag.number,
            offset: item.offset,
            octets: item.octets,
            body,
            header: header_len(item),
        }
    }
}

/// The length of `item`'s tag header: at most seven octets (Clause
/// 20.2.1), with an extended tag number and a four-octet length.
fn header_len(item: &TagItem<'_>) -> u8 {
    u8::try_from(item.octets.len().saturating_sub(item.contents.len())).unwrap_or(u8::MAX)
}

/// Read `data` whole as a tree of tags: the top-level elements in wire
/// order, each application primitive typed, each constructed element with
/// its members. Only with `alloc`.
///
/// The octets read are [`TagReader`]'s, so its faults are this function's,
/// and so is [`TagTreeFault::TooManyNodes`] for an element past
/// [`MAX_TAG_TREE_NODES`]. An application primitive that is well framed but
/// doesn't decode is a [`TagBody::Invalid`] node, not a fault. Empty `data`
/// is an empty tree.
///
/// ```
/// use bacnet_encoding::primitives::ApplicationValueRef;
/// use bacnet_encoding::tags::tree::{TagBody, decode_tag_tree};
///
/// // [0] 2, then [1] ( REAL 1.5 ).
/// let octets = [0x09, 0x02, 0x1E, 0x44, 0x3F, 0xC0, 0x00, 0x00, 0x1F];
/// let tree = decode_tag_tree(&octets)?;
/// assert_eq!(tree[0].body, TagBody::Context(&[0x02]));
/// assert_eq!(tree[1].octets, &octets[2..]);
/// assert_eq!(tree[1].children()[0].body, TagBody::Application(ApplicationValueRef::Real(1.5)));
///
/// // Cut short inside [1]: the error says where, and what read before it.
/// let error = decode_tag_tree(&octets[..6]).unwrap_err();
/// assert_eq!((error.offset, error.valid_up_to), (3, 2));
/// # Ok::<(), bacnet_encoding::tags::tree::TagTreeError>(())
/// ```
pub fn decode_tag_tree(data: &[u8]) -> Result<Vec<TagNode<'_>>, TagTreeError> {
    let mut top = Vec::new();
    // The open constructed elements, outermost first: each opening tag and
    // the members read so far.
    let mut open: Vec<(TagItem<'_>, Vec<TagNode<'_>>)> = Vec::new();
    let mut nodes = 0usize;
    let mut valid_up_to = 0;
    for item in TagReader::new(data) {
        let item = item?;
        let node = if item.tag.is_closing {
            let Some((opening, members)) = open.pop() else {
                // The reader only yields a closing tag it matched.
                return Err(TagTreeError {
                    fault: TagTreeFault::UnmatchedClosing {
                        number: item.tag.number,
                        open: None,
                    },
                    offset: item.offset,
                    valid_up_to,
                });
            };
            TagNode {
                class: TagClass::Context,
                number: opening.tag.number,
                offset: opening.offset,
                octets: data.get(opening.offset..item.end()).unwrap_or_default(),
                body: TagBody::Constructed(members),
                header: header_len(&opening),
            }
        } else {
            nodes += 1;
            if nodes > MAX_TAG_TREE_NODES {
                return Err(TagTreeError {
                    fault: TagTreeFault::TooManyNodes,
                    offset: item.offset,
                    valid_up_to,
                });
            }
            if item.tag.is_opening {
                open.push((item, Vec::new()));
                continue;
            }
            TagNode::primitive(&item)
        };
        match open.last_mut() {
            Some((_, members)) => members.push(node),
            None => {
                valid_up_to = item.end();
                top.push(node);
            }
        }
    }
    Ok(top)
}
