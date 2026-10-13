//! Selector parts against the semantic tree. The same code runs in every tier: tier 1 gets the
//! tree from `App::semantic_tree`, tier 3 from the `getTree` RPC.

use nd_core::semantic::{Role, SemanticNode, SemanticTree};

use crate::selector::Part;

/// True when `part` holds for `node` itself. Positional and structural parts are applied by
/// [`select`].
pub fn matches(node: &SemanticNode, part: &Part) -> bool {
    match part {
        Part::TestId(id) => node.test_id.as_deref() == Some(id.as_str()),
        Part::Type(kind) => node.kind == *kind,
        Part::Text(t) => node.role != Role::Window && !node.name.is_empty() && t.matches(&node.name),
        Part::Label(t) => node.role != Role::Window && t.matches(&node.name),
        Part::Placeholder(t) => t.matches(node.placeholder.as_deref().or(node.value.as_deref()).unwrap_or_default()),
        Part::HasText(t) => t.matches(&node.subtree_text()),
        Part::Role { role, name, checked, disabled } => {
            node.role.as_str() == role
                && name.as_ref().is_none_or(|n| n.matches(&node.name))
                && checked.is_none_or(|c| node.checked == Some(c))
                && disabled.is_none_or(|d| node.enabled != d)
        }
        Part::Nth(_) | Part::Has(_) | Part::HasNot(_) | Part::And(_) => true,
    }
}

fn descendants<'a>(node: &'a SemanticNode, out: &mut Vec<&'a SemanticNode>) {
    for child in &node.children {
        out.push(child);
        descendants(child, out);
    }
}

fn self_and_descendants(node: &SemanticNode) -> Vec<&SemanticNode> {
    let mut out = vec![node];
    descendants(node, &mut out);
    out
}

fn matches_all(node: &SemanticNode, parts: &[Part]) -> bool {
    parts.iter().all(|p| matches(node, p))
}

/// Resolves parts against `roots` (one per window). An engine part searches the roots and
/// everything under them the first time and strictly inside the previous matches after;
/// `nth`, `has`, `has-not`, `and` and `has-text` filter the current set. A selector with no
/// engine part matches nothing.
pub fn select<'a>(roots: &[&'a SemanticNode], parts: &[Part]) -> Vec<&'a SemanticNode> {
    let mut current: Vec<&SemanticNode> = roots.to_vec();
    let mut descended = false;
    for part in parts {
        match part {
            Part::HasText(_) => current.retain(|n| matches(n, part)),
            Part::Nth(i) => {
                let at = if *i < 0 { current.len() as i64 + i } else { *i };
                current = usize::try_from(at).ok().and_then(|at| current.get(at).copied()).into_iter().collect();
            }
            Part::And(inner) => current.retain(|n| matches_all(n, inner)),
            Part::Has(inner) | Part::HasNot(inner) => {
                let want = matches!(part, Part::Has(_));
                current.retain(|n| {
                    let mut under = Vec::new();
                    descendants(n, &mut under);
                    under.iter().any(|d| matches_all(d, inner)) == want
                });
            }
            engine => {
                let mut pool: Vec<&SemanticNode> = Vec::new();
                for scope in &current {
                    let candidates = if descended {
                        let mut under = Vec::new();
                        descendants(scope, &mut under);
                        under
                    } else {
                        self_and_descendants(scope)
                    };
                    for node in candidates {
                        if !pool.iter().any(|p| p.id == node.id) {
                            pool.push(node);
                        }
                    }
                }
                current = pool.into_iter().filter(|n| matches(n, engine)).collect();
                descended = true;
            }
        }
    }
    if descended { current } else { Vec::new() }
}

/// The window roots of `tree`, optionally only one window's.
pub fn roots(tree: &SemanticTree, window: Option<u64>) -> Vec<&SemanticNode> {
    tree.windows.iter().filter(|w| window.is_none_or(|id| w.id.0 == id)).map(|w| &w.root).collect()
}

/// Up to `limit` nodes whose name, test id or role is closest to `want`, for a locator error.
pub fn near_misses(tree: &SemanticTree, want: &str, limit: usize) -> Vec<String> {
    let want = want.to_lowercase();
    let mut scored: Vec<(usize, String)> = tree
        .nodes()
        .filter(|(_, n)| n.role != Role::Window && n.role != Role::Group || n.test_id.is_some())
        .map(|(_, n)| {
            let fields = [n.name.to_lowercase(), n.test_id.clone().unwrap_or_default().to_lowercase()];
            let score = fields
                .iter()
                .filter(|f| !f.is_empty())
                .map(|f| if f.contains(&want) || want.contains(f.as_str()) { 0 } else { distance(f, &want) })
                .min()
                .unwrap_or(usize::MAX);
            let mut line = format!("{} {:?}", n.role, n.name);
            if let Some(id) = &n.test_id {
                line.push_str(&format!(" [testid={id}]"));
            }
            line.push_str(&format!(" [ref=e{}]", n.id.0));
            (score, line)
        })
        .collect();
    scored.sort();
    scored.into_iter().take(limit).map(|(_, line)| line).collect()
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}
