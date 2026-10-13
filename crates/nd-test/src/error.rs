use std::fmt;

use nd_automation::{RpcError, RpcErrorCode};

#[derive(Debug)]
pub enum Error {
    /// The app answered with an RPC error.
    Rpc(RpcError),
    /// The socket broke or the app is gone.
    Io(std::io::Error),
    /// The app could not be started or connected to.
    Launch(String),
    /// A selector string that does not parse.
    Selector(String),
    /// More than one node matched an action or assertion that needs exactly one.
    Strict { selector: String, count: usize, matches: Vec<String> },
    /// The locator found nothing actionable before its timeout. Carries the reason for the
    /// last attempt and the nearest candidates.
    Timeout { selector: String, waited_ms: u128, reason: String, candidates: Vec<String> },
    /// An `expect` matcher that did not hold before its timeout.
    Assertion(String),
}

impl Error {
    pub(crate) fn is_stale(&self) -> bool {
        matches!(self, Error::Rpc(e) if e.kind() == Some(RpcErrorCode::NotActionable))
    }
}

impl From<RpcError> for Error {
    fn from(e: RpcError) -> Self {
        Error::Rpc(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Rpc(e) => write!(f, "automation: {e}"),
            Error::Io(e) => write!(f, "automation socket: {e}"),
            Error::Launch(e) => write!(f, "launch: {e}"),
            Error::Selector(e) => f.write_str(e),
            Error::Strict { selector, count, matches } => {
                write!(f, "strict mode: {selector} matched {count} nodes")?;
                for m in matches {
                    write!(f, "\n  {m}")?;
                }
                Ok(())
            }
            Error::Timeout { selector, waited_ms, reason, candidates } => {
                write!(f, "{selector}: {reason} after {waited_ms} ms")?;
                if !candidates.is_empty() {
                    f.write_str("\nnearest nodes:")?;
                    for c in candidates {
                        write!(f, "\n  {c}")?;
                    }
                }
                Ok(())
            }
            Error::Assertion(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}
