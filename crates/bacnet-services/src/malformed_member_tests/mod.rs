//! One table per decoder of malformed service parameters (#1374, #1375):
//! a member under the wrong class or tag number, a member cut short, and
//! octets after the last member, each with the kind of refusal it draws.
//!
//! Each table starts with a well-formed input, and every other row changes
//! that input in one place. A member whose contents stop before its header
//! says is [`Error::BufferTooShort`]; a wrong tag, a fixed-size member of the
//! wrong length and a member cut short inside a constructed frame (found
//! while the frame is extracted) are [`Error::Decoding`].

use bacnet_types::enums::{ErrorClass, ErrorCode, RejectReason};
use bacnet_types::error::{Error, RemoteError};

mod device;
mod discovery;
mod file;
mod object;
mod summary;

/// What a decoder makes of one input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// It decodes.
    Decodes,
    /// [`Error::Decoding`].
    Malformed,
    /// [`Error::BufferTooShort`].
    Short,
    /// [`Error::reject`] with this reason.
    Reject(RejectReason),
    /// [`Error::protocol`] with this class and code.
    Protocol(ErrorClass, ErrorCode),
    /// Any other error.
    Other,
}

use Kind::{Decodes, Malformed, Short};

impl Kind {
    fn of(result: &Result<(), Error>) -> Self {
        match result {
            Ok(()) => Decodes,
            Err(Error::Decoding(_)) => Malformed,
            Err(Error::BufferTooShort { .. }) => Short,
            Err(Error::Remote(RemoteError::Reject { reason })) => Kind::Reject(*reason),
            Err(Error::Remote(RemoteError::Error { class, code })) => Kind::Protocol(*class, *code),
            Err(_) => Kind::Other,
        }
    }
}

/// A decoder's result with the decoded value dropped.
type Decode = fn(&[u8]) -> Result<(), Error>;

/// `(row, input, expected kind)`.
type Row<'a> = (&'a str, &'a [u8], Kind);

macro_rules! decoder {
    ($ty:ty) => {
        (
            stringify!($ty),
            (|d: &[u8]| <$ty>::decode(d).map(drop)) as Decode,
        )
    };
}
use decoder;

/// Run every row through `decoder`, then fail listing each row whose kind
/// differs from the one expected.
fn check((name, decode): (&str, Decode), rows: &[Row<'_>]) {
    let failures: Vec<String> = rows
        .iter()
        .filter_map(|&(row, input, expected)| {
            let result = decode(input);
            (Kind::of(&result) != expected).then(|| {
                format!("{name} {row} {input:02X?}: expected {expected:?}, got {result:?}")
            })
        })
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// `parts` joined into one input.
fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}
