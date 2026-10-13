//! Stand-in driver: `examples/hello` mounted on `TestBackend`, its retained tree read back as a
//! semantic tree. It is real view code and a real reconciler, but no window and no automation
//! socket, so it only exists until `nd-automation` can attach to a launched host.

use nd::backend::NodeKind;
use nd::prelude::*;
use nd::test::{ButtonProps, NodeId, Size, TestApp, TextProps};

use crate::driver::{Driver, DriverError, Launcher, Node};

pub struct HelloStub;

impl Launcher for HelloStub {
    fn launch(&mut self) -> Result<Box<dyn Driver>, DriverError> {
        let mut app = TestApp::new();
        app.open_window(Size { width: 800., height: 600. }, |_, cx| cx.new(|_| hello::Counter::default()));
        Ok(Box::new(StubDriver { app }))
    }

    fn describe(&self) -> String {
        "stub: in-process tree of examples/hello on TestBackend (no window)".into()
    }
}

struct StubDriver {
    app: TestApp,
}

impl StubDriver {
    /// Containers carry no semantics, so their children hoist into the parent.
    fn convert(&self, id: NodeId) -> Vec<Node> {
        let backend = self.app.backend();
        let children: Vec<Node> = backend.children(id).iter().flat_map(|&child| self.convert(child)).collect();
        let (Some(kind), Some(props)) = (backend.kind(id), backend.props(id)) else {
            return children;
        };
        let mut node = Node { id: id.0 as u32, enabled: true, ..Node::default() };
        match kind {
            NodeKind::Root => {
                node.role = "window".into();
                node.name = Some("Hello".into());
            }
            NodeKind::Text => {
                node.role = "text".into();
                node.name = props.downcast_ref::<TextProps>().map(|t| t.text.to_string());
            }
            NodeKind::Widget(_) if props.downcast_ref::<ButtonProps>().is_some() => {
                node.role = "button".into();
                node.name = props.downcast_ref::<ButtonProps>().map(|b| b.label.to_string());
            }
            _ => return children,
        }
        node.children = children;
        vec![node]
    }
}

impl Driver for StubDriver {
    fn tree(&mut self) -> Result<Node, DriverError> {
        let root = self
            .app
            .backend()
            .find(|kind, _| kind == NodeKind::Root)
            .first()
            .copied()
            .ok_or("the window has no root")?;
        self.convert(root).into_iter().next().ok_or_else(|| "the window has no root".into())
    }

    fn click(&mut self, node: u32) -> Result<(), DriverError> {
        let tree = self.tree()?;
        match tree.find(node) {
            Some(found) if found.role == "button" => {
                self.app.emit(NodeId(u64::from(node)), ClickEvent);
                Ok(())
            }
            Some(found) => Err(format!("e{node} is a {}, not clickable", found.role).into()),
            None => Err(format!("no node e{node} in the tree").into()),
        }
    }

    fn alive(&mut self) -> bool {
        true
    }
}
