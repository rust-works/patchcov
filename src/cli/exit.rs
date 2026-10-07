//! Process exit codes: one per class of failure.
//!
//! A command fails with an [`anyhow::Error`]. The CLI layer tags the failures it
//! can tell apart with an [`ExitKind`] (see [`Classify`]), and `main` turns the tag
//! into the process exit code with [`code`]. The tag is invisible in the message,
//! so what is printed to stderr does not depend on it.

use std::error::Error as StdError;
use std::fmt;

/// A class of failure, and the exit code it ends the process with.
///
/// `2` is also what the argument parser exits with, so it is the code of every
/// usage error. The values are part of the command-line contract and listed in
/// `docs/reference.md#exit-codes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[non_exhaustive]
pub enum ExitKind {
    /// A gate failed: `--fail-under-patch`, `--fail-under-lines` or
    /// `--fail-on-unmeasured`.
    Gate = 1,
    /// A usage error the argument parser cannot see, such as a flag combination
    /// that makes no sense.
    Usage = 2,
    /// A coverage report is unreadable, empty or unparseable.
    Report = 3,
    /// A coverage marker in the source is malformed.
    Marker = 4,
    /// The config file, a path mapping, an ignore regex or a glob is bad.
    Config = 5,
    /// Git could not answer: no repository, an unresolvable ref, no merge base.
    Git = 6,
    /// A report's paths match no tracked file, unless `--allow-path-mismatch` is set.
    PathMismatch = 7,
    /// Any other runtime failure, such as an I/O error writing the output.
    Other = 8,
}

impl ExitKind {
    /// The process exit code for this class of failure.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// A new error with this class and `message`, for a check that fails outright.
    pub fn error(
        self,
        message: impl fmt::Display + fmt::Debug + Send + Sync + 'static,
    ) -> anyhow::Error {
        anyhow::Error::new(ExitError {
            kind: self,
            source: anyhow::Error::msg(message),
        })
    }
}

/// An error tagged with the [`ExitKind`] that decides the exit code.
///
/// It displays as the error it wraps and continues that error's chain, so
/// `{err:#}` prints the same text with or without the tag. The wrapped error
/// itself is no longer reachable by downcast, only its causes are.
#[derive(Debug)]
pub struct ExitError {
    kind: ExitKind,
    source: anyhow::Error,
}

impl ExitError {
    /// The class of failure.
    pub const fn kind(&self) -> ExitKind {
        self.kind
    }
}

impl fmt::Display for ExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.source, f)
    }
}

impl StdError for ExitError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        // `anyhow::Error` derefs to `dyn Error`; skip it so it is not named twice.
        let inner: &(dyn StdError + Send + Sync + 'static) = self.source.as_ref();
        inner.source()
    }
}

/// The class `err` was tagged with, if any.
fn kind_of(err: &anyhow::Error) -> Option<ExitKind> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<ExitError>())
        .map(ExitError::kind)
}

/// The exit code for `err`: its tag, or [`ExitKind::Other`] when it has none.
pub fn code(err: &anyhow::Error) -> u8 {
    kind_of(err).unwrap_or(ExitKind::Other).code()
}

/// Tags a failure with its [`ExitKind`].
pub trait Classify<T> {
    /// Tags the error, unless something deeper already did: the innermost class is
    /// the most specific one, so a caller can wrap broadly without overwriting it.
    fn classify(self, kind: ExitKind) -> anyhow::Result<T>;
}

impl<T, E> Classify<T> for Result<T, E>
where
    E: Into<anyhow::Error>,
{
    fn classify(self, kind: ExitKind) -> anyhow::Result<T> {
        self.map_err(|err| {
            let err = err.into();
            if kind_of(&err).is_some() {
                err
            } else {
                anyhow::Error::new(ExitError { kind, source: err })
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use anyhow::{anyhow, Context};

    use super::*;

    #[test]
    fn error_builds_a_tagged_error() {
        let err = ExitKind::Gate.error("it failed");
        assert_eq!(code(&err), 1);
        assert_eq!(format!("{err:#}"), "it failed");
    }

    fn fail(kind: ExitKind) -> anyhow::Error {
        Err::<(), _>(anyhow!("boom")).classify(kind).unwrap_err()
    }

    /// The numbers are a contract with scripts; this is the table in the docs.
    #[test]
    fn codes_are_stable() {
        let expected = [
            (ExitKind::Gate, 1),
            (ExitKind::Usage, 2),
            (ExitKind::Report, 3),
            (ExitKind::Marker, 4),
            (ExitKind::Config, 5),
            (ExitKind::Git, 6),
            (ExitKind::PathMismatch, 7),
            (ExitKind::Other, 8),
        ];
        for (kind, number) in expected {
            assert_eq!(kind.code(), number, "{kind:?}");
            assert_eq!(code(&fail(kind)), number, "{kind:?}");
        }
    }

    #[test]
    fn an_untagged_error_is_other() {
        assert_eq!(code(&anyhow!("boom")), ExitKind::Other.code());
    }

    /// Context added after the tag (as `?` through callers does) hides nothing.
    #[test]
    fn the_tag_survives_added_context() {
        let err = Err::<(), _>(fail(ExitKind::Git))
            .context("while resolving")
            .unwrap_err();
        assert_eq!(code(&err), ExitKind::Git.code());
    }

    #[test]
    fn the_innermost_class_wins() {
        let err = Err::<(), _>(fail(ExitKind::Report))
            .classify(ExitKind::Other)
            .unwrap_err();
        assert_eq!(code(&err), ExitKind::Report.code());
        let err = Err::<(), _>(fail(ExitKind::Report))
            .context("outer")
            .classify(ExitKind::Config)
            .unwrap_err();
        assert_eq!(code(&err), ExitKind::Report.code());
    }

    /// The tag must not change a character of what is printed.
    #[test]
    fn the_message_and_chain_are_unchanged() {
        let build = || {
            Err::<(), _>(std::io::Error::other("disk on fire"))
                .context("could not read it")
                .context("outer")
                .unwrap_err()
        };
        let plain = build();
        let tagged = Err::<(), _>(build())
            .classify(ExitKind::Report)
            .unwrap_err();
        assert_eq!(format!("{plain:#}"), format!("{tagged:#}"));
        assert_eq!(format!("{plain}"), format!("{tagged}"));
        assert_eq!(plain.chain().count(), tagged.chain().count());
    }

    /// A plain `std` error converts as well as an `anyhow` one.
    #[test]
    fn a_std_error_can_be_classified() {
        let err = Err::<(), _>(std::io::Error::other("x"))
            .classify(ExitKind::Other)
            .unwrap_err();
        assert_eq!(code(&err), ExitKind::Other.code());
        assert_eq!(err.to_string(), "x");
    }
}
