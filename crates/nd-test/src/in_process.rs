//! Tier 1: locators against an in-process `TestApp`. Requests run through the same
//! `nd_automation::handle` the socket server runs, then the app settles, so a locator test
//! written here reads the same against a real app in tier 3.

use std::cell::{RefCell, RefMut};
use std::ops::Deref;
use std::rc::Rc;

use nd_automation::{RpcError, RpcErrorCode, WaitParams, handle, wait_check};
use nd_ui::test_app::TestApp;
use serde_json::{Value, json};

use crate::Result;
use crate::session::{Driver, Session};

struct InProcess {
    app: Rc<RefCell<TestApp>>,
}

impl Driver for InProcess {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut app = self.app.try_borrow_mut().expect("a TestHarness call while its app() is borrowed");
        if method == "waitFor" {
            let wait = WaitParams::parse(&params)?;
            // No clock to wait on: the app is settled, so the condition holds now or never.
            app.run_until_idle();
            return match wait_check(&app, &wait)? {
                Some(result) => Ok(result),
                None => Err(RpcError::new(RpcErrorCode::WaitForTimeout, "waitFor timeout")
                    .with_data(json!({ "timeoutMs": wait.timeout_ms() }))
                    .into()),
            };
        }
        let result = handle(&mut app, method, &params);
        app.run_until_idle();
        Ok(result?)
    }

    fn tick(&self) -> bool {
        if let Ok(mut app) = self.app.try_borrow_mut() {
            app.run_until_idle();
        }
        false
    }
}

/// A tier 1 app: a `TestApp` with the locator API.
///
/// ```ignore
/// let mut app = TestApp::new();
/// app.open_window(size, |_, cx| cx.new(|_| Counter::default()));
/// let app = TestHarness::new(app);
/// app.get_by_test_id("add").click()?;
/// expect(&app.get_by_text("1").exact()).to_be_visible()?;
/// ```
pub struct TestHarness {
    app: Rc<RefCell<TestApp>>,
    session: Session,
}

impl TestHarness {
    pub fn new(app: TestApp) -> Self {
        let app = Rc::new(RefCell::new(app));
        let session = Session::new(Rc::new(InProcess { app: app.clone() }));
        Self { app, session }
    }

    /// The app itself, for state and backend assertions. Drop the borrow before the next
    /// locator call.
    pub fn app(&self) -> RefMut<'_, TestApp> {
        self.app.borrow_mut()
    }
}

impl Deref for TestHarness {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.session
    }
}
