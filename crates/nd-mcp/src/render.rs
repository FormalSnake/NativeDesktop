//! The compact tree an agent reads before acting: one line per node, indented by depth, with the
//! wire ref every action targets.

use crate::driver::Node;

pub const MAX_LINES: usize = 400;

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderOptions {
    /// Keep only nodes an action can reach (and the ancestors that lead to them).
    pub interactive_only: bool,
}

const INTERACTIVE: &[&str] = &["button", "textbox", "checkbox", "switch", "slider", "link", "menuitem", "tab", "row"];

pub fn is_interactive(node: &Node) -> bool {
    INTERACTIVE.contains(&node.role.as_str())
}

fn keeps(node: &Node, options: RenderOptions) -> bool {
    !options.interactive_only || is_interactive(node) || node.children.iter().any(|c| keeps(c, options))
}

pub fn render_snapshot(root: &Node, options: RenderOptions) -> String {
    let mut lines = Vec::new();
    let mut truncated = 0;
    emit(root, 0, options, &mut lines, &mut truncated);
    if truncated > 0 {
        lines.push(format!("... {truncated} more nodes"));
    }
    lines.join("\n")
}

fn emit(node: &Node, depth: usize, options: RenderOptions, lines: &mut Vec<String>, truncated: &mut usize) {
    if !keeps(node, options) {
        return;
    }
    if lines.len() >= MAX_LINES {
        *truncated += 1;
    } else {
        lines.push(line(node, depth));
    }
    for child in &node.children {
        emit(child, depth + 1, options, lines, truncated);
    }
}

fn line(node: &Node, depth: usize) -> String {
    let mut out = format!("{}- {}", "  ".repeat(depth), node.role);
    if let Some(name) = &node.name {
        out.push_str(&format!(" {name:?}"));
    }
    if let Some(value) = &node.value {
        out.push_str(&format!(" value={value:?}"));
    }
    if let Some(test_id) = &node.test_id {
        out.push_str(&format!(" testid={test_id:?}"));
    }
    if !node.enabled {
        out.push_str(" [disabled]");
    }
    if node.focused {
        out.push_str(" [focused]");
    }
    out.push_str(&format!(" [ref=e{}]", node.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, role: &str, name: Option<&str>, children: Vec<Node>) -> Node {
        Node { id, role: role.into(), name: name.map(Into::into), enabled: true, children, ..Node::default() }
    }

    #[test]
    fn renders_one_indented_line_per_node_with_refs() {
        let tree = node(1, "window", Some("Hello"), vec![node(2, "button", Some("Add one"), vec![])]);
        assert_eq!(
            render_snapshot(&tree, RenderOptions::default()),
            "- window \"Hello\" [ref=e1]\n  - button \"Add one\" [ref=e2]"
        );
    }

    #[test]
    fn interactive_only_keeps_paths_to_actionable_nodes() {
        let tree = node(
            1,
            "window",
            None,
            vec![
                node(2, "text", Some("0"), vec![]),
                node(3, "group", None, vec![node(4, "button", Some("Go"), vec![])]),
            ],
        );
        let out = render_snapshot(&tree, RenderOptions { interactive_only: true });
        assert_eq!(out.lines().count(), 3);
        assert!(!out.contains("\"0\""));
    }

    #[test]
    fn long_trees_are_capped() {
        let tree = node(0, "list", None, (1..=500).map(|i| node(i, "row", None, vec![])).collect());
        let out = render_snapshot(&tree, RenderOptions::default());
        assert_eq!(out.lines().count(), MAX_LINES + 1);
        assert!(out.ends_with("... 101 more nodes"));
    }
}
