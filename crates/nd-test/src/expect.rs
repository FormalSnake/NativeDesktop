//! `expect(&locator)` matchers. Each polls the locator's match set until the condition holds
//! or the timeout passes; `.not()` inverts. A matcher about one node is strict: more than one
//! match fails at once.

use std::time::{Duration, Instant};

use nd_core::semantic::SemanticNode;

use crate::locator::{Locator, Resolved};
use crate::selector::TextMatch;
use crate::{Error, Result};

pub fn expect(locator: &Locator) -> LocatorAssertions {
    LocatorAssertions { locator: locator.clone(), negate: false, timeout: None }
}

pub struct LocatorAssertions {
    locator: Locator,
    negate: bool,
    timeout: Option<Duration>,
}

/// What a check saw: whether it held, and what to say when it did not.
struct Seen {
    holds: bool,
    actual: String,
}

impl LocatorAssertions {
    #[allow(clippy::should_implement_trait)]
    pub fn not(mut self) -> Self {
        self.negate = !self.negate;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    fn poll(&self, expected: &str, check: impl Fn(&Resolved) -> Result<Seen>) -> Result<()> {
        let timeout = self.timeout.unwrap_or_else(|| self.locator.timeout());
        let start = Instant::now();
        let mut settled = false;
        loop {
            let resolved = self.locator.resolve()?;
            let seen = check(&resolved)?;
            if seen.holds != self.negate {
                return Ok(());
            }
            if start.elapsed() >= timeout || settled {
                let not = if self.negate { "not " } else { "" };
                return Err(Error::Assertion(format!(
                    "expect({}) to {not}{expected}, got {} after {} ms\n{}",
                    self.locator,
                    seen.actual,
                    start.elapsed().as_millis(),
                    resolved.tree.snapshot()
                )));
            }
            settled = !self.locator.session().tick();
        }
    }

    /// Checks the single match; no match fails (or passes, negated, where `absent` says so).
    fn one(&self, expected: &str, absent: bool, check: impl Fn(&Resolved, &SemanticNode) -> Seen) -> Result<()> {
        self.poll(expected, |resolved| match resolved.matches.as_slice() {
            [] => Ok(Seen { holds: absent, actual: "no match".into() }),
            [node] => Ok(check(resolved, node)),
            many => Err(Error::Strict {
                selector: self.locator.selector(),
                count: many.len(),
                matches: many.iter().map(|n| format!("{} {:?} [ref=e{}]", n.role, n.name, n.id.0)).collect(),
            }),
        })
    }

    pub fn to_be_visible(&self) -> Result<()> {
        self.one("be visible", false, |r, n| Seen { holds: r.visible(n), actual: "hidden".into() })
    }

    /// Holds when nothing matches or the match is not visible.
    pub fn to_be_hidden(&self) -> Result<()> {
        self.one("be hidden", true, |r, n| Seen { holds: !r.visible(n), actual: "visible".into() })
    }

    pub fn to_be_attached(&self) -> Result<()> {
        self.poll("be attached", |r| Ok(Seen { holds: !r.matches.is_empty(), actual: "no match".into() }))
    }

    pub fn to_be_enabled(&self) -> Result<()> {
        self.one("be enabled", false, |_, n| Seen { holds: n.enabled, actual: "disabled".into() })
    }

    pub fn to_be_disabled(&self) -> Result<()> {
        self.one("be disabled", false, |_, n| Seen { holds: !n.enabled, actual: "enabled".into() })
    }

    pub fn to_be_checked(&self) -> Result<()> {
        self.one("be checked", false, |_, n| Seen {
            holds: n.checked == Some(true),
            actual: format!("{:?}", n.checked),
        })
    }

    pub fn to_be_focused(&self) -> Result<()> {
        self.one("be focused", false, |_, n| Seen { holds: n.focused, actual: "not focused".into() })
    }

    /// The node's text (its accessible name, or for a container the text under it) equals
    /// `text` after whitespace normalisation.
    pub fn to_have_text(&self, text: &str) -> Result<()> {
        let want = TextMatch::Exact(text.to_owned());
        self.one(&format!("have text {text:?}"), false, |_, n| {
            let actual = if n.name.is_empty() { n.subtree_text() } else { n.name.clone() };
            Seen { holds: want.matches(&actual), actual: format!("{actual:?}") }
        })
    }

    /// The node's text contains `text`, case-insensitively.
    pub fn to_contain_text(&self, text: &str) -> Result<()> {
        let want = TextMatch::Substring(text.to_owned());
        self.one(&format!("contain text {text:?}"), false, |_, n| {
            let actual = n.subtree_text();
            Seen { holds: want.matches(&actual), actual: format!("{actual:?}") }
        })
    }

    pub fn to_have_value(&self, value: &str) -> Result<()> {
        self.one(&format!("have value {value:?}"), false, |_, n| Seen {
            holds: n.value.as_deref() == Some(value),
            actual: format!("{:?}", n.value),
        })
    }

    pub fn to_have_count(&self, count: usize) -> Result<()> {
        self.poll(&format!("have count {count}"), |r| {
            Ok(Seen { holds: r.matches.len() == count, actual: r.matches.len().to_string() })
        })
    }
}
