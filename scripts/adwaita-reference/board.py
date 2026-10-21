"""The libadwaita reference boards nd's Linux look is compared against.

`python3 board.py <page> <light|dark>` opens one board in real libadwaita, with hover, press and
focus forced on the controls that show them, so `ndshell shot` captures every state in one frame.
The pages mirror the boards in crates/nd-backend-gpui/tests/hig.rs control for control.
"""

import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gio, GLib, Gtk  # noqa: E402

PAGE, SCHEME = sys.argv[1], sys.argv[2]
HOVER = Gtk.StateFlags.PRELIGHT
ACTIVE = Gtk.StateFlags.ACTIVE
FOCUS = Gtk.StateFlags.FOCUSED | Gtk.StateFlags.FOCUS_VISIBLE
forced = []


def force(widget, flags):
    forced.append((widget, flags))
    return widget


def box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12, *children):
    b = Gtk.Box(orientation=orientation, spacing=spacing)
    for c in children:
        b.append(c)
    return b


def row(*children):
    b = box(Gtk.Orientation.HORIZONTAL, 12, *children)
    b.set_valign(Gtk.Align.CENTER)
    return b


def column(spacing, *children):
    return box(Gtk.Orientation.VERTICAL, spacing, *children)


def button(label=None, icon=None, classes=(), flags=None):
    b = Gtk.Button()
    if label:
        b.set_label(label)
    if icon:
        b.set_icon_name(icon)
    for c in classes:
        b.add_css_class(c)
    b.set_valign(Gtk.Align.CENTER)
    return force(b, flags) if flags else b


def buttons(flags):
    toggle = Gtk.ToggleButton(label="On", active=True, valign=Gtk.Align.CENTER)
    if flags:
        force(toggle, flags)
    return row(
        button("Button", flags=flags),
        button("Save", classes=["suggested-action"], flags=flags),
        button("Delete", classes=["destructive-action"], flags=flags),
        button("Flat", classes=["flat"], flags=flags),
        button(icon="open-menu-symbolic", classes=["flat"], flags=flags),
        button(icon="list-add-symbolic", flags=flags),
        button(icon="edit-delete-symbolic", classes=["circular"], flags=flags),
        button("Pill", classes=["pill"], flags=flags),
        toggle,
    )


def entry(placeholder="", text="", flags=None, classes=()):
    e = Gtk.Entry(placeholder_text=placeholder, text=text, width_chars=12, valign=Gtk.Align.CENTER)
    for c in classes:
        e.add_css_class(c)
    return force(e, flags) if flags else e


def check(label, active=False, inconsistent=False, sensitive=True, flags=None, group=None):
    c = Gtk.CheckButton(label=label, active=active, inconsistent=inconsistent, sensitive=sensitive)
    if group:
        c.set_group(group)
    return force(c, flags) if flags else c


def switch(active, flags=None):
    s = Gtk.Switch(active=active, valign=Gtk.Align.CENTER)
    return force(s, flags) if flags else s


def controls():
    search = Gtk.SearchEntry(placeholder_text="Search", valign=Gtk.Align.CENTER)
    password = Gtk.PasswordEntry(placeholder_text="Password", show_peek_icon=True, valign=Gtk.Align.CENTER)
    radio = check("One", active=True)
    toggles = Adw.ToggleGroup(valign=Gtk.Align.CENTER)
    for name in ["Day", "Week", "Month"]:
        toggles.add(Adw.Toggle(label=name, name=name))
    toggles.set_active_name("Day")
    linked = row(button("Left"), button("Right"))
    linked.set_spacing(0)
    linked.add_css_class("linked")
    dropdown = Gtk.DropDown.new_from_strings(["Name", "Date", "Size"])
    dropdown.set_selected(1)
    dropdown.set_valign(Gtk.Align.CENTER)
    spin = Gtk.SpinButton.new_with_range(0, 10, 0.5)
    spin.set_value(2.5)
    spin.set_valign(Gtk.Align.CENTER)
    scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 100, 1)
    scale.set_value(25)
    scale.set_draw_value(False)
    scale.set_size_request(180, -1)
    scale.set_valign(Gtk.Align.CENTER)
    for mark in range(0, 101, 25):
        scale.add_mark(mark, Gtk.PositionType.BOTTOM, None)
    hovered_scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 100, 1)
    hovered_scale.set_value(60)
    hovered_scale.set_draw_value(False)
    hovered_scale.set_size_request(140, -1)
    hovered_scale.set_valign(Gtk.Align.CENTER)
    force(hovered_scale, HOVER)
    progress = Gtk.ProgressBar(fraction=0.4, valign=Gtk.Align.CENTER)
    progress.set_size_request(140, -1)
    link = Gtk.LinkButton(uri="https://example.org", label="Read the docs", valign=Gtk.Align.CENTER)
    return column(
        14,
        buttons(None),
        buttons(HOVER),
        buttons(ACTIVE),
        buttons(FOCUS),
        row(
            entry("Name"),
            entry(text="Typed", flags=FOCUS),
            password,
            search,
            entry(text="Wrong", classes=["error"]),
        ),
        row(
            check("Check"),
            check("Checked", active=True),
            check("Mixed", inconsistent=True),
            check("Disabled", sensitive=False),
            check("Hover", flags=HOVER),
            check("Focus", active=True, flags=FOCUS),
            radio,
            check("Two", group=radio),
        ),
        row(
            switch(False),
            switch(True),
            switch(False, HOVER),
            switch(True, HOVER),
            switch(False, FOCUS),
            toggles,
            linked,
            dropdown,
            spin,
        ),
        row(scale, hovered_scale, progress, Adw.Spinner(), link),
    )


def action_row(title, subtitle=None, suffix=None, activatable=False, flags=None):
    r = Adw.ActionRow(title=title)
    if subtitle:
        r.set_subtitle(subtitle)
    if suffix:
        r.add_suffix(suffix)
    if activatable:
        r.add_suffix(Gtk.Image(icon_name="go-next-symbolic"))
        r.set_activatable(True)
    return force(r, flags) if flags else r


def lists():
    group = Adw.PreferencesGroup(title="Appearance", description="How windows look")
    sw = Gtk.Switch(active=True, valign=Gtk.Align.CENTER)
    group.add(action_row("Dark style", "Follow the system", sw))
    group.add(Adw.SwitchRow(title="Animations", active=False))
    combo = Adw.ComboRow(title="Accent", model=Gtk.StringList.new(["Blue", "Teal", "Pink"]))
    group.add(combo)
    group.add(Adw.EntryRow(title="Name", text="Kyan"))
    group.add(action_row("Hovered row", activatable=True, flags=HOVER))
    group.add(action_row("Pressed row", activatable=True, flags=ACTIVE))
    expander = Adw.ExpanderRow(title="More", subtitle="Expands", expanded=True)
    expander.add_row(Adw.ActionRow(title="Inside"))
    group.add(expander)
    second = Adw.PreferencesGroup(title="About")
    second.add(action_row("Version", suffix=Gtk.Label(label="1.0", css_classes=["dim-label"])))
    page = Adw.PreferencesPage()
    page.add(group)
    page.add(second)
    return page


def sidebar_list():
    lb = Gtk.ListBox(selection_mode=Gtk.SelectionMode.SINGLE)
    lb.add_css_class("navigation-sidebar")
    entries = [
        ("mail-inbox-symbolic", "Inbox", "12"),
        ("folder-symbolic", "Work", None),
        ("document-edit-symbolic", "Q3 planning", None),
        ("airplane-mode-symbolic", "Trips", None),
        ("starred-symbolic", "Starred", None),
    ]
    rows = []
    for icon, title, count in entries:
        b = row(Gtk.Image(icon_name=icon), Gtk.Label(label=title, xalign=0, hexpand=True))
        if count:
            b.append(Gtk.Label(label=count, css_classes=["dim-label"]))
        r = Gtk.ListBoxRow(child=b)
        lb.append(r)
        rows.append(r)
    lb.select_row(rows[2])
    force(rows[3], HOVER)
    return lb


def app():
    split = Adw.NavigationSplitView()
    side_view = Adw.ToolbarView()
    side_view.add_top_bar(Adw.HeaderBar(title_widget=Adw.WindowTitle(title="Library")))
    side_view.set_content(Gtk.ScrolledWindow(child=sidebar_list()))
    split.set_sidebar(Adw.NavigationPage(title="Library", child=side_view))
    content = Adw.ToolbarView()
    header = Adw.HeaderBar(title_widget=Adw.WindowTitle(title="Q3 planning", subtitle="Edited today"))
    header.pack_start(button(icon="list-add-symbolic"))
    header.pack_end(Gtk.MenuButton(icon_name="open-menu-symbolic"))
    header.pack_end(button(icon="system-search-symbolic", flags=HOVER))
    content.add_top_bar(header)
    banner = Adw.Banner(title="An update is ready to install", button_label="Restart", revealed=True)
    content.add_top_bar(banner)
    overlay = Adw.ToastOverlay(child=lists())
    toast = Adw.Toast(title="Note deleted", button_label="Undo", timeout=0)
    GLib.timeout_add(100, lambda: overlay.add_toast(toast) and False)
    content.set_content(overlay)
    split.set_content(Adw.NavigationPage(title="Q3 planning", child=content))
    return split


def dialog(window):
    content = Adw.ToolbarView()
    content.add_top_bar(Adw.HeaderBar())
    content.set_content(Gtk.Label(label="Inbox", vexpand=True))
    alert = Adw.AlertDialog(heading="Delete the note?", body="The note and its attachments are removed for good.")
    alert.add_response("cancel", "Cancel")
    alert.add_response("delete", "Delete")
    alert.set_response_appearance("delete", Adw.ResponseAppearance.DESTRUCTIVE)
    GLib.timeout_add(200, lambda: alert.present(window) and False)
    return content


def popover():
    menu = Gio.Menu()
    menu.append("New Window", "app.new")
    menu.append("Preferences", "app.prefs")
    section = Gio.Menu()
    section.append("Keyboard Shortcuts", "app.keys")
    section.append("About Notes", "app.about")
    menu.append_section(None, section)
    button = Gtk.MenuButton(icon_name="open-menu-symbolic", menu_model=menu)
    header = Adw.HeaderBar()
    header.pack_end(button)
    view = Adw.ToolbarView()
    view.add_top_bar(header)
    view.set_content(Gtk.Label(label="", vexpand=True))
    GLib.timeout_add(200, lambda: button.popup() and False)
    return view


def tabs():
    view = Adw.TabView()
    for title in ["Inbox", "Drafts", "Settings"]:
        page = view.append(Gtk.Label(label=title))
        page.set_title(title)
    view.set_selected_page(view.get_nth_page(1))
    bar = Adw.TabBar(view=view, autohide=False)
    content = Adw.ToolbarView()
    content.add_top_bar(Adw.HeaderBar())
    content.add_top_bar(bar)
    content.set_content(view)
    return content


def typography():
    styles = [
        ("large-title", "Large title"),
        ("title-1", "Title 1"),
        ("title-2", "Title 2"),
        ("title-3", "Title 3"),
        ("title-4", "Title 4"),
        ("heading", "Heading"),
        ("body", "Body text reads at the default size"),
        ("caption-heading", "Caption heading"),
        ("caption", "Caption"),
        ("dim-label", "Dimmed body"),
        ("monospace", "monospace 0123"),
    ]
    col = column(8, *[Gtk.Label(label=text, xalign=0, css_classes=[cls]) for cls, text in styles])
    status = Adw.StatusPage(
        icon_name="document-new-symbolic", title="No Notes", description="Create a note to get started"
    )
    status.set_child(button("New Note", classes=["pill", "suggested-action"]))
    status.set_size_request(420, -1)
    return row(col, status)


def on_activate(application):
    # A private test session has no settings portal; GNOME's default interface font.
    Gtk.Settings.get_default().set_property("gtk-font-name", "Adwaita Sans 11")
    Adw.StyleManager.get_default().set_color_scheme(
        Adw.ColorScheme.FORCE_DARK if SCHEME == "dark" else Adw.ColorScheme.FORCE_LIGHT
    )
    window = Adw.ApplicationWindow(application=application, title="Reference")
    sizes = {
        "controls": (1000, 640),
        "app": (960, 620),
        "dialog": (800, 560),
        "popover": (480, 360),
        "tabs": (800, 300),
        "typography": (960, 520),
    }
    if PAGE == "dialog":
        body = dialog(window)
    else:
        body = {"controls": controls, "app": app, "popover": popover, "tabs": tabs, "typography": typography}[PAGE]()
    if PAGE in ("controls", "typography"):
        body.set_margin_top(16)
        body.set_margin_bottom(16)
        body.set_margin_start(16)
        body.set_margin_end(16)
        view = Adw.ToolbarView(content=body)
        view.add_top_bar(Adw.HeaderBar())
        body = view
    window.set_content(body)
    window.set_default_size(*sizes[PAGE])
    window.present()

    def apply():
        for widget, flags in forced:
            widget.set_state_flags(flags, False)
        return True

    GLib.timeout_add(100, apply)


application = Adw.Application(application_id="dev.nativedesktop.AdwaitaReference", flags=Gio.ApplicationFlags.NON_UNIQUE)
application.connect("activate", on_activate)
application.run([])
