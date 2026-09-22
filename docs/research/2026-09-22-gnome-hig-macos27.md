# GNOME HIG pass and macOS 27 notes (2026-09-22)

## What landed

GTK layout. Buttons, switches, pickers and the other self-sized kinds no longer
stretch to a vertical box's width or a horizontal box's height. The table is
`nd_self_sized_kinds` in `tools/codegen.ts`, the same list as Layout.swift's
`ndSelfSizedKinds` minus Label, Checkbox and Radio (a wrapping label needs the
width; Adwaita gives check rows the whole hit area). `src/gtk/style.zig` now
treats hexpand/vexpand/halign/valign as set-replace: a dropped key falls back
to the kind default, and `hexpand: true` with no halign fills.

Header bars. Children insert by sibling on both backends. GTK packs one GtkBox
per slot (AdwHeaderBar's pack_start/pack_end only append); AppKit's
`ndHeaderBarPack` takes the `before` view. The GTK end slot is now tree order
left to right, as on AppKit. The notes example's pin/delete/new buttons flip
accordingly.

Box placement. An `insertBefore` whose anchor was an unpacked Popover or Dialog
sibling used to send the new child to the front of the box. `ndBoxPlace`
appends in that case.

HIG fixes from the audit: Banner and Toast no longer parse Pango markup (an `&`
blanked them); StatusPage descriptions are escaped; sourcelist row icons are
symbolic; the SplitView breakpoint is in sp, not px; `<spinner>` is AdwSpinner;
`dialog.showMessage` uses AdwAlertDialog like `showAlert` already did; a
tooltip also sets the GTK accessible name; the button size classes are
`nd-button-small` / `nd-button-large` (libadwaita owns `.compact`).

New props on both backends: `<button destructive>`, `<label variant>`
(title1..title4, heading, body, caption, captionHeading, monospace),
`<settingsgroup separateRows>` (GTK only, AppKit keeps the inset form), and
`dim-label` in the cssClasses allowlist.

## Verified

`zig build`, `zig build test` and `swift build` pass. AppKit: the notes,
propreset, popover-anchor and settings drives are green. GTK on this mac
(quartz): notes and propreset are green; popover-anchor and settings fail the
same way on an untouched main checkout (waitFor timeout, screenshot invalid
params), so that is the macOS GTK host, not this change. The Linux weston gates
(`scripts/headless-*.sh`) did not run here.

## GTK follow-ups from the HIG audit, not done

- View switcher belongs in the header bar title slot, with an
  AdwViewSwitcherBar at the bottom under 550sp (`<tabview>` today draws an
  inline switcher above the stack).
- `<splitview>` is always AdwOverlaySplitView; a navigation sidebar wants
  AdwNavigationSplitView (25% width, 180 to 280sp, collapses to a stack).
- No AdwNavigationView / AdwNavigationPage kinds at all.
- Boxed lists only have `<row>` and `<switchrow>`; EntryRow, PasswordEntryRow,
  ComboRow, SpinRow, ExpanderRow and ButtonRow are missing, so a `<textinput>`
  in a `<settingsgroup>` lands below the list.
- `<row activatable>` defaults false while `activated` is always connected; the
  host does not know which listeners an app attached, so deriving it needs a
  wire hint.
- `<listview>` rows are bare labels with hand-set margins.
- Window has no minWidth/minHeight or breakpoints.
- Banner `button-style`, AdwBottomSheet for `<sheet edge="bottom">`, AdwWrapBox
  for tag rows.

## macOS 27 (Golden Gate, public 2026-09-14)

Verified against Apple's macOS 27 release notes and WWDC26 session 289:
`NSToolbarItemGroup.role` and `NSSegmentedControl.role` (`.tabs` gets a
distinct look and VoiceOver reads it as tabs); `NSMenuItem.preferredImageVisibility`
and menus hiding symbol images by default (non-symbol images too when linked
against the 27 SDK); `NSViewCornerConfiguration` with
`containerConcentric`; titlebar accessories drawing outside their bounds on the
27 SDK; gesture recognizer exclusivity on by default
(`NSView.exclusiveGestureBehavior` opts out); `NSRefreshController`;
`NSTextSelectionManager`. Xcode 27 ships Swift 6.4 and needs macOS 26.6 to
run. GitHub has no `macos-27` runner label; the image is `xcode-27`, a public
preview on a 27 beta build.

Nothing in the AppKit shell changed for 27 in this pass. This machine runs
macOS 26.5, so no 27 API compiles or runs here, and CI stays on `macos-26`
until `xcode-27` leaves preview. The existing KVC write of
`preferredImageVisibility` in MenuBar.swift already covers 27's menu image
default; the shell only uses symbol images.

To do once CI links the 27 SDK, each behind `#available(macOS 27.0, *)`:

- Set `NSToolbarItemGroup.role` on grouped runs and
  `NSSegmentedControl.role = .tabs` on the nav control
  (`swift/Sources/NDShell/HeaderBar.swift:444` and `:1222`).
- Replace the `NDRadius` literals (`Metrics.swift:15`, used by toasts and the
  command palette) with `cornerConfiguration = .uniformCorners(radius:
  .containerConcentric(min))`, keeping the literals as the 26 fallback.
- Re-check the split sidebar and the `.soft` scroll edge
  (`SplitController.swift:328`) on a 27 machine: 27 reverted the floating inset
  sidebar to flush left with a darker background. The shell leans on
  NSSplitViewItem's sidebar behaviour, so this is likely a visual check only.
- Run `scripts/mac/mac-gestures.sh` on 27 before trusting MAC_GESTURES_OK; the
  posted NSEvent batches may hit the new gesture exclusivity.

Unconfirmed, secondary sources only: Icon Composer 2 changes to the `.icon`
JSON (`packages/nd/src/package/iconcomposer.ts`), SF Symbols 8 renames. The HIG
what's-new pages could not be fetched, so no HIG page was read first hand.
