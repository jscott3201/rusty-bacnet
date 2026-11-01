//! What the encoding crate's fuzz targets check, one module per target, and
//! the round-trip check they share.
//!
//! Every target takes any octets. Besides its own oracle, each one relies on
//! libFuzzer for the rest of what a decoder of untrusted input owes: no
//! panic, no allocation past libFuzzer's limit (2,048 megabytes by default)
//! and no recursion deep enough to overflow the stack.
//!
//! The seed inputs in `seeds/<target>/` were harvested from the unit tests
//! or are built by the `seeds` module; `cargo test`, which `fuzz.sh check`
//! runs, replays every one through its target's check.

use std::fmt::Debug;

use bacnet_types::error::Error;
use bytes::BytesMut;

/// Declares `CODECS`, `run`, which runs the codec a selector picks, and
/// (for tests) `decodes`.
/// An entry is `name: decode => encode`, where the encoder is a closure of
/// the value and the buffer that returns a `Result`; `name: decode` alone
/// only decodes.
macro_rules! codecs {
    ($($name:literal: $decode:expr $(=> $encode:expr)?;)*) => {
        /// The codecs, in selector order.
        pub const CODECS: &[&str] = &[$($name),*];

        fn run(selector: usize, input: &[u8]) {
            let mut index = 0;
            $(
                if selector == index {
                    codecs!(@one $name, input, $decode $(, $encode)?);
                    return;
                }
                index += 1;
            )*
            let _ = index;
        }

        /// Whether the codec at `selector` decodes `input`: what the
        /// generated seeds are held to.
        #[cfg(test)]
        pub(crate) fn decodes(selector: usize, input: &[u8]) -> bool {
            let mut index = 0;
            $(
                if selector == index {
                    return $decode(input).is_ok();
                }
                index += 1;
            )*
            let _ = index;
            false
        }
    };
    (@one $name:literal, $input:ident, $decode:expr, $encode:expr) => {
        $crate::round_trip($name, $input, $decode, $encode)
    };
    (@one $name:literal, $input:ident, $decode:expr) => {{
        if let Ok((_, end)) = $decode($input) {
            assert!(end <= $input.len(), "{}: decoded past the input's end", $name);
        }
    }};
}

/// An infallible encoder taking the buffer first, as a closure for
/// `codecs!`.
macro_rules! infallible {
    ($encode:path) => {
        |value, buf: &mut bytes::BytesMut| {
            $encode(buf, value);
            Ok(())
        }
    };
}

/// A fallible encoder taking the buffer first, as a closure for
/// `codecs!`.
macro_rules! buf_first {
    ($encode:path) => {
        |value, buf: &mut bytes::BytesMut| $encode(buf, value)
    };
}

/// A fallible encoder taking the value first, as a closure for
/// `codecs!`.
macro_rules! value_first {
    ($encode:path) => {
        |value, buf: &mut bytes::BytesMut| $encode(value, buf)
    };
}

pub mod apdu;
pub mod constructed;
pub mod service;
pub mod value;

#[cfg(test)]
mod seeds;

/// Splits an input into a selector octet and the octets after it; an empty
/// input selects 0 with nothing after it.
pub fn select(input: &[u8]) -> (u8, &[u8]) {
    match input.split_first() {
        Some((&selector, rest)) => (selector, rest),
        None => (0, &[]),
    }
}

/// The round trip a decoder and its encoder owe each other: when `decode`
/// accepts `input`, ending at or before its end, and `encode` takes the
/// value, `decode` accepts those octets whole, to the same value, and it
/// encodes again to the same octets.
///
/// Values are compared by their `Debug` text, not with `==`: a value may
/// hold a NaN, which equals nothing, and its text matches itself. An encoder
/// may refuse what its decoder accepts (a decoder is lenient where Clause 20
/// lets it be, an encoder strict), so a refusal ends the check.
pub fn round_trip<T: Debug>(
    what: &str,
    input: &[u8],
    decode: impl Fn(&[u8]) -> Result<(T, usize), Error>,
    encode: impl Fn(&T, &mut BytesMut) -> Result<(), Error>,
) {
    let Ok((value, end)) = decode(input) else {
        return;
    };
    assert!(
        end <= input.len(),
        "{what}: decoded past the input's end ({end} of {} octets)",
        input.len()
    );
    let mut first = BytesMut::new();
    if encode(&value, &mut first).is_err() {
        return;
    }
    let (again, end) = decode(&first).unwrap_or_else(|error| {
        panic!("{what}: the encoding of {value:?}, {first:02x?}, doesn't decode: {error:?}")
    });
    assert_eq!(
        end,
        first.len(),
        "{what}: the encoding of {value:?}, {first:02x?}, decodes only in part"
    );
    assert_eq!(
        format!("{value:?}"),
        format!("{again:?}"),
        "{what}: the value changed through its encoding {first:02x?}"
    );
    let mut second = BytesMut::new();
    encode(&again, &mut second).unwrap_or_else(|error| {
        panic!("{what}: {again:?}, decoded from {first:02x?}, doesn't encode: {error:?}")
    });
    assert_eq!(
        first, second,
        "{what}: {value:?} encodes to {first:02x?}, which decodes to {again:?}, which encodes to {second:02x?}"
    );
}

/// A decoder of the whole input, as [`round_trip`] takes one.
pub fn whole<T>(
    decode: impl Fn(&[u8]) -> Result<T, Error>,
) -> impl Fn(&[u8]) -> Result<(T, usize), Error> {
    move |data| decode(data).map(|value| (value, data.len()))
}

/// A decoder that starts at offset 0 and reports where it stopped, as
/// [`round_trip`] takes one.
pub fn at<T>(
    decode: impl Fn(&[u8], usize) -> Result<(T, usize), Error>,
) -> impl Fn(&[u8]) -> Result<(T, usize), Error> {
    move |data| decode(data, 0)
}
