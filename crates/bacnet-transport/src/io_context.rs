//! An I/O failure with what the transport was doing when it failed (#1833):
//! its text leads with that, and its kind and its source are the failure's
//! own, so an OS error code stays reachable through `source()`.

use std::fmt;
use std::io;

use bacnet_types::error::Error;

/// `error` as an I/O error of its kind whose message starts with `context`
/// and whose `source()` is `error` itself.
pub(crate) fn context(error: io::Error, context: impl Into<String>) -> io::Error {
    io::Error::new(
        error.kind(),
        ContextError {
            context: context.into(),
            error,
        },
    )
}

/// The operating system's refusal to give entropy for `what`, as the I/O
/// error the entropy library makes of it.
pub(crate) fn entropy(error: getrandom::Error, what: &str) -> Error {
    Error::Transport(context(io::Error::from(error), what))
}

#[derive(Debug)]
struct ContextError {
    context: String,
    error: io::Error,
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.context, self.error)
    }
}

impl std::error::Error for ContextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure `error` wraps: the source of its payload.
    fn wrapped(error: &io::Error) -> &io::Error {
        let source = error.get_ref().and_then(|payload| payload.source());
        source.and_then(|source| source.downcast_ref()).unwrap()
    }

    /// The context leads the text; the kind and the OS code stay the
    /// failure's own.
    #[test]
    fn context_keeps_the_kind_and_the_os_error() {
        let os = io::Error::from_raw_os_error(2);
        let text = os.to_string();
        let error = context(os, "Serial open failed for /dev/none");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(
            error.to_string(),
            format!("Serial open failed for /dev/none: {text}")
        );
        assert_eq!(wrapped(&error).raw_os_error(), Some(2));
    }

    /// An entropy failure is a transport error whose source is the entropy
    /// library's own error, worded with what it was for.
    #[test]
    fn entropy_is_a_transport_error() {
        let Error::Transport(io) = entropy(getrandom::Error::UNSUPPORTED, "no VMAC") else {
            panic!("not a transport error");
        };
        assert_eq!(io.kind(), io::ErrorKind::Other);
        assert!(io.to_string().starts_with("no VMAC: "), "{io}");
        let library = wrapped(&io).get_ref().unwrap();
        assert!(library.downcast_ref::<getrandom::Error>().is_some());
    }
}
