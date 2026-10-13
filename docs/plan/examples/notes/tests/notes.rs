//! Tier 1 (logic) and tier 2 (pixels) run in-process on the test backend and the headless
//! renderer; tier 3 launches the real app and drives it over the automation socket.

use nd::prelude::*;
use nd::test::*;
use notes::{app::Notes, demo};

fn open(cx: &mut TestApp) -> TestWindow {
    cx.update(demo::seed);
    cx.open_window(WindowOptions::main("main"), |window, cx| cx.new(|cx| Notes::new(window, cx)))
}

#[nd::test]
async fn pinned_notes_sort_first(cx: &mut TestApp) -> nd::Result<()> {
    let win = open(cx);
    let rows = win.get_by_role(Role::ListItem).within(win.get_by_test_id("notes"));
    expect(rows.first()).to_contain_text("Groceries").await?;
    expect(rows).to_have_count(4).await?;
    Ok(())
}

#[nd::test]
async fn new_note_is_selected_and_focused(cx: &mut TestApp) -> nd::Result<()> {
    let win = open(cx);
    win.get_by_role(Role::Button).name("New Note").click().await?;
    let first = win.get_by_role(Role::ListItem).first();
    expect(&first).to_contain_text("Untitled").await?;
    expect(&first).to_be_selected().await?;
    win.keyboard().type_text("Plan the week\nMonday: ship").await?;
    expect(first).to_contain_text("Plan the week").await?;
    Ok(())
}

#[nd::test]
async fn delete_from_context_menu_keeps_scroll_anchor(cx: &mut TestApp) -> nd::Result<()> {
    let win = open(cx);
    let trip = win.get_by_test_id("note-2");
    let before = trip.bounds().await?.top();
    let removed = win.get_by_test_id("note-1").bounds().await?.size.height;
    win.get_by_test_id("note-1").click_with(MouseButton::Right).await?;
    win.get_by_role(Role::MenuItem).name("Delete").click().await?;
    expect(win.get_by_test_id("note-1")).to_be_gone().await?;
    // The removed row was above; the list moves up by exactly its height, nothing jumps.
    assert_eq!(trip.bounds().await?.top(), before - removed);
    Ok(())
}

#[nd::test]
async fn palette_completes_and_opens_a_note(cx: &mut TestApp) -> nd::Result<()> {
    let win = open(cx);
    win.keyboard().press("secondary-k").await?;
    let palette = win.get_by_role(Role::Dialog).name("Command Palette");
    expect(&palette).to_be_visible().await?;
    win.keyboard().type_text("lis").await?;
    expect(palette.get_by_role(Role::SearchBox)).to_have_value("lis|bon").await?; // completion shown selected
    win.keyboard().press("tab").await?;
    win.keyboard().press("enter").await?;
    expect(palette).to_be_gone().await?;
    expect(win.get_by_test_id("editor")).to_contain_text("Tram 28").await?;
    Ok(())
}

#[nd::test]
async fn settings_sheet_round_trips_to_toml(cx: &mut TestApp) -> nd::Result<()> {
    let win = open(cx);
    win.keyboard().press("secondary-,").await?;
    win.get_by_role(Role::Switch).name("Compact rows").click().await?;
    win.get_by_role(Role::Button).name("Done").click().await?;
    let toml = std::fs::read_to_string(cx.config_dir().join("config.toml"))?;
    assert!(toml.contains("compact_rows = true"));
    Ok(())
}

#[nd::test]
async fn pinch_zoom_snaps_at_one_hundred(cx: &mut TestApp) -> nd::Result<()> {
    let win = open(cx);
    let editor = win.get_by_test_id("editor");
    editor.pinch(0.8).await?;
    editor.pinch(1.3).await?;
    expect(win.get_by_test_id("zoom-label")).to_be_gone().await?; // crossed 100%, snapped back
    assert_eq!(cx.haptics(), [Haptic::Alignment]);
    Ok(())
}

#[nd::test(pixels)]
async fn default_window(cx: &mut TestApp) -> nd::Result<()> {
    let win = open(cx);
    win.set_size(size(px(960.), px(600.)));
    win.snapshot("notes-default").await?; // golden per platform, light and dark
    Ok(())
}

#[nd::test(real)]
async fn swipe_to_delete_on_the_real_window() -> nd::Result<()> {
    let app = launch(env!("CARGO_BIN_EXE_notes")).env("NOTES_DEMO", "1").start().await?;
    let row = app.get_by_test_id("note-3");
    row.swipe(Direction::Left).await?; // trackpad gesture on macOS, synthesized at the platform layer elsewhere
    app.get_by_role(Role::Button).name("Delete").click().await?;
    expect(row).to_be_gone().await?;
    app.record().last_frames(30).save("swipe-delete")?; // kept as a test artifact for review
    Ok(())
}

#[nd::test(real)]
async fn popover_menu_does_not_leak_into_the_editor() -> nd::Result<()> {
    let app = launch(env!("CARGO_BIN_EXE_notes")).env("NOTES_DEMO", "1").start().await?;
    app.get_by_test_id("note-2").click_with(MouseButton::Right).await?;
    let editor = app.get_by_test_id("editor");
    // Outside press closes the menu and is consumed; the editor never sees it.
    app.mouse().click(editor.center().await?).await?;
    expect(app.get_by_role(Role::Menu)).to_be_gone().await?;
    expect(editor).not().to_be_focused().await?;
    Ok(())
}
