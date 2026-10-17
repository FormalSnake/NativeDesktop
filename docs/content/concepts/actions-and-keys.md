+++
title = "Actions and keys"
description = "Declare actions, bind keys in contexts, build menus from the same actions, and move focus."
+++

A command is an action. A key binding, a menu item, and a toolbar button each name an action, and a
handler on the view responds to it. Because all three go through the same dispatch, a shortcut and
its menu item never disagree.

<!-- docs:example chrome -->

## Declare actions

<!-- docs:snippet examples/chrome/src/lib.rs#actions -->

`actions!(namespace, [Name, ..])` declares unit structs that implement `Action`. The prelude
includes it.

## Handle them

A handler is an `on_action` listener on an element, plus a `key_context` that names the part of the
tree the bindings apply to:

<!-- docs:snippet examples/chrome/src/lib.rs#dispatch -->

An action starts at the focused element and goes up through its ancestors, and the first `on_action`
listener for that type stops it. A handler that wants the action to continue calls
`cx.propagate()`. `cx.listener(Self::delete)`
accepts a method with the signature `fn(&mut self, &Action, &mut Window, &mut Context<Self>)`, and a
closure takes the same arguments. A handler that needs no view state can take `cx` alone:
`|_: &OpenSettings, _, cx| ..`.

Toolbar and row buttons dispatch an action instead of running a closure:
`icon_button("new", Icon::SquarePen).dispatch(NewMessage)`. The button calls `dispatch_action` on its
window, so the action reaches the same listeners a key press would.

## Bind keys

<!-- docs:snippet examples/chrome/src/lib.rs#menus -->

`KeyBinding::new("secondary-n", NewMessage, Some("Mail"))` binds a keystroke to an action while focus
is inside an element whose `key_context` is `"Mail"`. `None` binds everywhere. `secondary` means Cmd
on macOS and Ctrl elsewhere, so one binding follows each platform's convention. Call
`cx.bind_keys` once at startup, before or after the window opens. Windows that are already open pick
up the bindings.

Before its own bindings, every app also gets Tab and Shift+Tab (`FocusNext`, `FocusPrevious`),
Return (`Confirm`), and Escape (`Cancel`). If no listener handles Return, it activates the default
button of the innermost focus scope, unless the focused element keeps Return for itself
(`takes_return()`, as a multi-line text view does). If no listener handles Escape, it dismisses the
innermost focus scope that asked for it, which is the innermost overlay.

## Menus

`set_menus(cx, [Menu::app([..]), Menu::new("File").items([..]), ..])`, in the same snippet, builds the
menu bar from the same actions. `MenuItem::action("New Message", NewMessage)` shows the key
equivalent of the matching binding, and `.destructive()` marks the item. `Menu::app` and `Menu::help`
mark the platform's application and help menus. On macOS, the result is the real menu bar, and the
web backend draws one. The Linux and Windows GPUI backend doesn't show menus yet, so `set_menus` does
nothing there. The key bindings work on every backend.

## Focus

The framework owns focus on every backend. Native controls report to it when they become first
responder, and it tells AppKit which view to make first responder.

- `.focusable()` makes an element a focus target. Tab visits targets in tree order. `.tab_index(n)`
  moves one earlier, and `.tab_stop(false)` keeps it out of the Tab order while code can still
  focus it.
- `.focus_scope(FocusScope { trap, dismiss_on_escape })` makes a subtree own focus while it is
  mounted. Mounting saves the previous focus, unmounting restores it, and `trap` keeps Tab inside.
  Overlays, sheets and the command palette are scopes.
- `cx.focus(window_id, node)` moves focus to a node by id, and `cx.focused(window_id)` reads it.
  There's no `FocusHandle` yet, so a view that must focus a control it renders gives that control a
  stable id.
- Focus rings show after keyboard input and hide after a pointer press.

A key binding gets the first turn, before the focused text control sees the keystroke, so a
shortcut works while a text field has focus.

## Test it

Dispatch runs the same code on the test backend, so a tier 1 test can call
`cx.dispatch_action(window_id, &NewMessage)` or `cx.dispatch_keystroke(window_id, &keystroke)` and
assert on the tree. Clicking a toolbar button by id goes through the same path.
