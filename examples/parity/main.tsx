import { render, useState, Spacing, ContentMargin, openExternal } from "@nativedesktop/react";
import type { SourceTreeNode, TableColumn, TableRow } from "@nativedesktop/react";
import {
  Accordion,
  DescriptionList,
  Pagination,
  Stepper,
  HoverCard,
  SearchableList,
  Form,
  FormField,
  OtpInput,
  ButtonGroup,
  StatusBar,
} from "@nativedesktop/ui";
import type {
  DescriptionListItem,
  SearchableListItem,
  ButtonGroupItem,
  StepperStep,
} from "@nativedesktop/ui";
import { DockView, useDock, seedDock, TilesView, useTiles, seedTiles } from "@nativedesktop/panes";
import type { DockModel, DockZone, TileModel } from "@nativedesktop/panes";

// gpui-component parity gallery (docs/plans/gpui-parity.md): a showcase for
// the three pieces wave 1 + 2 + panes landed: new native widgets (avatar,
// badge, tag, kbd, combobox, breadcrumb, tooltip, levelindicator styles),
// @nativedesktop/ui's composition layer, and @nativedesktop/panes' Dock and
// Tiles models. Navigation follows examples/notes' two-pane <splitview>
// shape: a <sourcetree> sidebar (native nav-sidebar semantics, not a
// hand-rolled one) selects which section renders in the content pane.
// Every control is wired to real state; see the per-section comments for
// the handful of widgets that carry no change event of their own
// (LevelIndicator, Kbd) and are paired with a real control instead.

type SectionId =
  | "display"
  | "input"
  | "navigation"
  | "composition"
  | "data"
  | "overlays"
  | "richtext"
  | "charts"
  | "progress"
  | "loading"
  | "dock"
  | "tiles"
  | "dragdrop"
  | "codeeditor";

const sectionTitles: Record<SectionId, string> = {
  display: "Display",
  input: "Input",
  navigation: "Navigation",
  composition: "Composition",
  data: "Data",
  overlays: "Overlays",
  richtext: "Rich Text",
  charts: "Charts",
  progress: "Progress",
  loading: "Loading",
  dock: "Dock",
  tiles: "Tiles",
  dragdrop: "Drag and Drop",
  codeeditor: "Code Editor",
};

const navNodes: SourceTreeNode[] = [
  { id: "display", title: "Display", iconName: "view-grid-symbolic", testID: "nav-display" },
  { id: "input", title: "Input", iconName: "input-keyboard-symbolic", testID: "nav-input" },
  { id: "navigation", title: "Navigation", iconName: "go-next-symbolic", testID: "nav-navigation" },
  { id: "composition", title: "Composition", iconName: "applications-utilities-symbolic", testID: "nav-composition" },
  { id: "data", title: "Data", iconName: "view-list-symbolic", testID: "nav-data" },
  { id: "overlays", title: "Overlays", iconName: "window-new-symbolic", testID: "nav-overlays" },
  { id: "richtext", title: "Rich Text", iconName: "text-x-generic-symbolic", testID: "nav-richtext" },
  { id: "charts", title: "Charts", iconName: "view-statistics-symbolic", testID: "nav-charts" },
  { id: "progress", title: "Progress", iconName: "emblem-synchronizing-symbolic", testID: "nav-progress" },
  { id: "loading", title: "Loading", iconName: "content-loading-symbolic", testID: "nav-loading" },
  { id: "dock", title: "Dock", iconName: "sidebar-show-symbolic", testID: "nav-dock" },
  { id: "tiles", title: "Tiles", iconName: "view-app-grid-symbolic", testID: "nav-tiles" },
  { id: "dragdrop", title: "Drag and Drop", iconName: "input-mouse-symbolic", testID: "nav-dragdrop" },
  { id: "codeeditor", title: "Code Editor", iconName: "accessories-text-editor-symbolic", testID: "nav-codeeditor" },
];

type Variant = "neutral" | "accent" | "success" | "warning" | "error";
const badgeVariants: Variant[] = ["neutral", "accent", "success", "warning", "error"];

interface TagChip {
  id: string;
  label: string;
  variant: Variant;
}

// --- Display ---------------------------------------------------------------

function DisplaySection(): React.ReactNode {
  const [avatarName, setAvatarName] = useState("Ada Lovelace");
  const [badgeVariantIndex, setBadgeVariantIndex] = useState(1);
  const [tags, setTags] = useState<TagChip[]>([
    { id: "t1", label: "React", variant: "accent" },
    { id: "t2", label: "Zig", variant: "warning" },
    { id: "t3", label: "Swift", variant: "success" },
  ]);
  const [nextTagId, setNextTagId] = useState(4);
  const [newTagLabel, setNewTagLabel] = useState("");
  // LevelIndicator has no change event of its own (it is a read-only
  // display widget, like ProgressBar), so each style below is paired with a
  // real control that owns the state and drives `value`.
  const [ratingValue, setRatingValue] = useState(3);
  const [continuousValue, setContinuousValue] = useState(0.6);
  const [discreteValue, setDiscreteValue] = useState(2);

  function addTag(): void {
    const label = newTagLabel.trim();
    if (label === "") return;
    const variant = badgeVariants[tags.length % badgeVariants.length]!;
    setTags((prev) => [...prev, { id: `t${nextTagId}`, label, variant }]);
    setNextTagId((n) => n + 1);
    setNewTagLabel("");
  }

  function removeTag(id: string): void {
    setTags((prev) => prev.filter((t) => t.id !== id));
  }

  return (
    <scrollview testID="display-scroll" style={{ vexpand: true }}>
      <box orientation="vertical" style={{ padding: ContentMargin }}>
        <settingsgroup title="Avatar" description="Text initials, live from the name below." testID="display-avatar-group">
          <row title="Name" testID="display-avatar-name-row">
            <textinput text={avatarName} onChanged={(e) => setAvatarName(e.text)} testID="display-avatar-input" />
          </row>
          <row title="Sizes" testID="display-avatar-sizes-row">
            <box orientation="horizontal" spacing={Spacing.sm}>
              <avatar text={avatarName} size={24} testID="display-avatar-24" />
              <avatar text={avatarName} size={32} testID="display-avatar-32" />
              <avatar text={avatarName} size={48} testID="display-avatar-48" />
              <avatar text={avatarName} size={64} testID="display-avatar-64" />
            </box>
          </row>
        </settingsgroup>

        <settingsgroup title="Badge" description="All five variants, plus a dot indicator." testID="display-badge-group">
          <row title="Live variant" testID="display-badge-variant-row">
            <select
              options={badgeVariants}
              selectedIndex={badgeVariantIndex}
              onSelectionChanged={(e) => setBadgeVariantIndex(e.index)}
              testID="display-badge-select"
            />
          </row>
          <row title="Preview" testID="display-badge-preview-row">
            <box orientation="horizontal" spacing={Spacing.xs}>
              <badge label="Live" variant={badgeVariants[badgeVariantIndex]} testID="display-badge-live" />
              {badgeVariants.map((v) => (
                <badge key={v} label={v} variant={v} testID={`display-badge-${v}`} />
              ))}
              <badge dot variant="accent" testID="display-badge-dot" />
            </box>
          </row>
        </settingsgroup>

        <settingsgroup title="Tag" description="Removable chips backed by real state." testID="display-tag-group">
          <row title="Add" testID="display-tag-add-row">
            <box orientation="horizontal" spacing={Spacing.sm}>
              <textinput text={newTagLabel} placeholder="New tag" onChanged={(e) => setNewTagLabel(e.text)} testID="display-tag-input" />
              <button label="Add" onClick={addTag} testID="display-tag-add-button" />
            </box>
          </row>
          <row title="Tags" testID="display-tag-list-row">
            <box orientation="horizontal" spacing={Spacing.xs}>
              {tags.map((t) => (
                <tag key={t.id} label={t.label} variant={t.variant} removable onRemoved={() => removeTag(t.id)} testID={`display-tag-${t.id}`} />
              ))}
              {tags.length === 0 && <label text="No tags" cssClasses={["dimmed"]} testID="display-tag-empty" />}
            </box>
          </row>
        </settingsgroup>

        <settingsgroup title="Kbd" description="Shortcut labels, paired with the command they trigger." testID="display-kbd-group">
          <row title="Command Palette" testID="display-kbd-palette-row">
            <kbd keys="⌘K" testID="display-kbd-palette" />
          </row>
          <row title="Save" testID="display-kbd-save-row">
            <kbd keys="⌘S" testID="display-kbd-save" />
          </row>
          <row title="Rename" testID="display-kbd-rename-row">
            <kbd keys="F2" testID="display-kbd-rename" />
          </row>
        </settingsgroup>

        <settingsgroup title="Level Indicator" description="Rating, continuous, and discrete styles, each driven by a real control." testID="display-level-group">
          <row title="Rating" testID="display-rating-row">
            <box orientation="horizontal" spacing={Spacing.sm}>
              <levelindicator indicatorStyle="rating" min={0} max={5} value={ratingValue} testID="display-rating-indicator" />
              <button label="-" onClick={() => setRatingValue((v) => Math.max(0, v - 1))} testID="display-rating-minus" />
              <button label="+" onClick={() => setRatingValue((v) => Math.min(5, v + 1))} testID="display-rating-plus" />
            </box>
          </row>
          <row title="Continuous" testID="display-continuous-row">
            <box orientation="horizontal" spacing={Spacing.sm} style={{ hexpand: true }}>
              <levelindicator
                indicatorStyle="continuous"
                min={0}
                max={1}
                value={continuousValue}
                style={{ hexpand: true }}
                testID="display-continuous-indicator"
              />
              <slider
                min={0}
                max={1}
                step={0.01}
                value={continuousValue}
                onValueChanged={(e) => setContinuousValue(e.value)}
                testID="display-continuous-slider"
              />
            </box>
          </row>
          <row title="Discrete" testID="display-discrete-row">
            <box orientation="horizontal" spacing={Spacing.sm}>
              <levelindicator indicatorStyle="discrete" min={0} max={5} value={discreteValue} testID="display-discrete-indicator" />
              <button label="-" onClick={() => setDiscreteValue((v) => Math.max(0, v - 1))} testID="display-discrete-minus" />
              <button label="+" onClick={() => setDiscreteValue((v) => Math.min(5, v + 1))} testID="display-discrete-plus" />
            </box>
          </row>
        </settingsgroup>
      </box>
    </scrollview>
  );
}

// --- Input -------------------------------------------------------------

const fruitOptions = ["Apple", "Banana", "Cherry", "Durian", "Elderberry"];

function InputSection(): React.ReactNode {
  const [selectIndex, setSelectIndex] = useState(0);
  const [comboIndex, setComboIndex] = useState(0);
  const [comboText, setComboText] = useState(fruitOptions[0]!);
  const [controlsEnabled, setControlsEnabled] = useState(true);
  const [enabledClickCount, setEnabledClickCount] = useState(0);

  return (
    <scrollview testID="input-scroll" style={{ vexpand: true }}>
      <box orientation="vertical" style={{ padding: ContentMargin }}>
        <settingsgroup
          title="Select vs ComboBox"
          description="Same option list, two pickers: Select only lets you choose, ComboBox also accepts free text."
          testID="input-group"
        >
          <row title="Select" subtitle="Fixed list, no typing" testID="input-select-row">
            <select options={fruitOptions} selectedIndex={selectIndex} onSelectionChanged={(e) => setSelectIndex(e.index)} testID="input-select" />
          </row>
          <row title="ComboBox" subtitle="Searchable and editable" testID="input-combobox-row">
            <combobox
              options={fruitOptions}
              selectedIndex={comboIndex}
              text={comboText}
              placeholder="Type or pick a fruit"
              onSelectionChanged={(e) => {
                setComboIndex(e.index);
                setComboText(fruitOptions[e.index] ?? comboText);
              }}
              onChanged={(e) => setComboText(e.text)}
              testID="input-combobox"
            />
          </row>
          <row title="Select value" testID="input-select-readout-row">
            <label text={fruitOptions[selectIndex] ?? "(none)"} cssClasses={["dimmed"]} testID="input-select-readout" />
          </row>
          <row title="ComboBox value" testID="input-combobox-readout-row">
            <label text={comboText || "(empty)"} cssClasses={["dimmed"]} testID="input-combobox-readout" />
          </row>
        </settingsgroup>

        <settingsgroup
          title="Enabled"
          description="The universal `enabled` prop, off on every control type at once."
          testID="input-enabled-group"
        >
          <row title="Controls enabled" testID="input-enabled-toggle-row">
            <switch checked={controlsEnabled} onToggled={(e) => setControlsEnabled(e.checked)} testID="input-enabled-toggle" />
          </row>
          <row title="Button" testID="input-enabled-button-row">
            <button label={`Clicked ${enabledClickCount}`} enabled={controlsEnabled} onClick={() => setEnabledClickCount((n) => n + 1)} testID="input-enabled-button" />
          </row>
          <row title="TextInput" testID="input-enabled-textinput-row">
            <textinput placeholder="Type here" enabled={controlsEnabled} testID="input-enabled-textinput" />
          </row>
          <row title="Select" testID="input-enabled-select-row">
            <select options={fruitOptions} selectedIndex={0} enabled={controlsEnabled} testID="input-enabled-select" />
          </row>
          <row title="Slider" testID="input-enabled-slider-row">
            <slider min={0} max={1} value={0.5} enabled={controlsEnabled} style={{ hexpand: true }} testID="input-enabled-slider" />
          </row>
        </settingsgroup>
      </box>
    </scrollview>
  );
}

// --- Navigation ------------------------------------------------------------

const breadcrumbPath = ["Home", "Documents", "Projects", "Parity"];

const stepperSteps: StepperStep[] = [
  { id: "account", title: "Account", description: "Create credentials" },
  { id: "profile", title: "Profile", description: "Add details" },
  { id: "review", title: "Review", description: "Check everything" },
  { id: "done", title: "Done" },
];

function NavigationSection(): React.ReactNode {
  const [breadcrumbIndex, setBreadcrumbIndex] = useState(breadcrumbPath.length - 1);
  const [page, setPage] = useState(1);
  const [stepIndex, setStepIndex] = useState(0);

  return (
    <scrollview testID="navigation-scroll" style={{ vexpand: true }}>
      <box orientation="vertical" style={{ padding: ContentMargin }}>
        <settingsgroup title="Breadcrumb" description="Click a segment to jump the path." testID="nav-breadcrumb-group">
          <row title="Path" testID="nav-breadcrumb-row">
            <breadcrumb items={breadcrumbPath} selectedIndex={breadcrumbIndex} onItemActivated={(e) => setBreadcrumbIndex(e.index)} testID="nav-breadcrumb" />
          </row>
          <row title="Current" testID="nav-breadcrumb-readout-row">
            <label text={breadcrumbPath[breadcrumbIndex] ?? ""} cssClasses={["dimmed"]} testID="nav-breadcrumb-readout" />
          </row>
        </settingsgroup>

        <settingsgroup title="Pagination" description="12 pages, one sibling around the current page." testID="nav-pagination-group">
          <row title="Page" testID="nav-pagination-row">
            <Pagination page={page} pageCount={12} onPageChange={setPage} testID="nav-pagination" />
          </row>
          <row title="Current" testID="nav-pagination-readout-row">
            <label text={`Page ${page} of 12`} cssClasses={["dimmed"]} testID="nav-pagination-readout" />
          </row>
        </settingsgroup>

        <settingsgroup title="Stepper" description="Click a step, or move with the buttons below." testID="nav-stepper-group">
          <row title="Steps" testID="nav-stepper-row">
            <Stepper steps={stepperSteps} activeIndex={stepIndex} onStepClick={setStepIndex} testID="nav-stepper" />
          </row>
          <row title="Controls" testID="nav-stepper-controls-row">
            <box orientation="horizontal" spacing={Spacing.sm}>
              <button label="Back" onClick={() => setStepIndex((i) => Math.max(0, i - 1))} testID="nav-stepper-back" />
              <button label="Next" onClick={() => setStepIndex((i) => Math.min(stepperSteps.length - 1, i + 1))} testID="nav-stepper-next" />
            </box>
          </row>
        </settingsgroup>
      </box>
    </scrollview>
  );
}

// --- Composition -------------------------------------------------------

interface AccordionSource {
  id: string;
  label: string;
  body: string;
}

const accordionSources: AccordionSource[] = [
  { id: "widgets", label: "New widgets", body: "Avatar, Badge, Tag, Kbd, ComboBox, Breadcrumb, plus a universal tooltip prop." },
  { id: "ui", label: "@nativedesktop/ui", body: "Ten pure-TypeScript components composed from existing native widgets." },
  { id: "panes", label: "@nativedesktop/panes", body: "Dock and Tiles model layouts on top of the existing paned/grid widgets." },
];

const buildInfo: DescriptionListItem[] = [
  { label: "Widgets added", value: "avatar, badge, tag, kbd, combobox, breadcrumb" },
  { label: "New prop", value: "tooltip (every widget), indicatorStyle (LevelIndicator)" },
  { label: "Target", value: "gpui-component parity, waves 1 and 2" },
];

const searchableItems: SearchableListItem[] = [
  { id: "accordion", label: "Accordion" },
  { id: "descriptionlist", label: "DescriptionList" },
  { id: "pagination", label: "Pagination" },
  { id: "stepper", label: "Stepper" },
  { id: "hovercard", label: "HoverCard" },
  { id: "searchablelist", label: "SearchableList" },
  { id: "form", label: "Form" },
  { id: "otpinput", label: "OtpInput" },
  { id: "buttongroup", label: "ButtonGroup" },
  { id: "statusbar", label: "StatusBar" },
];

const rangeItems: ButtonGroupItem[] = [
  { id: "day", label: "Day" },
  { id: "week", label: "Week" },
  { id: "month", label: "Month" },
];

function CompositionSection(): React.ReactNode {
  const [expandedIds, setExpandedIds] = useState<string[]>(["widgets"]);
  const [lastActivated, setLastActivated] = useState("(none)");
  // Seeded invalid on purpose so FormField's error state is visible without
  // requiring input first.
  const [email, setEmail] = useState("invalid-email");
  const [otp, setOtp] = useState("");
  const [otpStatus, setOtpStatus] = useState("Enter the 6-digit code");
  const [range, setRange] = useState("week");

  const emailError = email.length > 0 && !email.includes("@") ? "Must contain @" : undefined;

  return (
    <scrollview testID="composition-scroll" style={{ vexpand: true }}>
      <box orientation="vertical" style={{ padding: ContentMargin }}>
        <settingsgroup title="Accordion" testID="composition-accordion-group">
          <Accordion
            items={accordionSources.map((s) => ({
              id: s.id,
              label: s.label,
              content: <label text={s.body} cssClasses={["dimmed"]} style={{ padding: Spacing.sm }} />,
            }))}
            expandedIds={expandedIds}
            onExpandedChange={setExpandedIds}
            allowMultiple
            testID="composition-accordion"
          />
        </settingsgroup>

        <DescriptionList title="Build Info" items={buildInfo} testID="composition-description-list" />

        <settingsgroup title="SearchableList" testID="composition-searchable-group">
          <SearchableList
            items={searchableItems}
            onActivate={(item) => setLastActivated(item.label)}
            placeholder="Filter components"
            testID="composition-searchable"
          />
          <row title="Last activated" testID="composition-searchable-readout-row">
            <label text={lastActivated} cssClasses={["dimmed"]} testID="composition-searchable-readout" />
          </row>
        </settingsgroup>

        <Form title="Sign up" description="Validated inline, native settings-row chrome." testID="composition-form">
          <FormField
            label="Email"
            error={emailError}
            hint={emailError ? undefined : "We'll only use this for release notes"}
            testID="composition-form-email-field"
          >
            <textinput text={email} placeholder="you@example.com" onChanged={(e) => setEmail(e.text)} testID="composition-form-email-input" />
          </FormField>
        </Form>

        <settingsgroup title="OtpInput" testID="composition-otp-group">
          <row title="Verification code" testID="composition-otp-row">
            <OtpInput length={6} value={otp} onChange={setOtp} onComplete={(v) => setOtpStatus(`Code complete: ${v}`)} testID="composition-otp" />
          </row>
          <row title="Status" testID="composition-otp-status-row">
            <label text={otpStatus} cssClasses={["dimmed"]} testID="composition-otp-readout" />
          </row>
        </settingsgroup>

        <settingsgroup title="ButtonGroup" testID="composition-buttongroup-group">
          <row title="Range" testID="composition-buttongroup-row">
            <ButtonGroup items={rangeItems} selectedId={range} onPress={setRange} testID="composition-buttongroup" />
          </row>
        </settingsgroup>

        <settingsgroup title="HoverCard" testID="composition-hovercard-group">
          <row title="Hover the button" testID="composition-hovercard-row">
            <HoverCard testID="composition-hovercard" content={<label text="Extra detail shown on hover." style={{ padding: Spacing.sm }} />}>
              <button label="Hover for info" testID="composition-hovercard-anchor" />
            </HoverCard>
          </row>
        </settingsgroup>

        <StatusBar
          testID="composition-statusbar"
          left={<label text={`${expandedIds.length} accordion open`} cssClasses={["caption"]} />}
          center={<label text={`Range: ${range}`} cssClasses={["caption"]} />}
          right={<label text={otp.length > 0 ? `OTP ${otp.length}/6` : "OTP empty"} cssClasses={["caption"]} />}
        />
      </box>
    </scrollview>
  );
}

// --- Data ------------------------------------------------------------------

interface Contact {
  id: string;
  name: string;
  role: string;
  team: string;
}

const contacts: Contact[] = [
  { id: "c1", name: "Ada Lovelace", role: "Engineer", team: "Platform" },
  { id: "c2", name: "Grace Hopper", role: "Architect", team: "Compilers" },
  { id: "c3", name: "Alan Turing", role: "Researcher", team: "Security" },
  { id: "c4", name: "Margaret Hamilton", role: "Lead", team: "Flight Software" },
];

const contactColumns: TableColumn[] = [
  { id: "name", title: "Name" },
  { id: "role", title: "Role" },
  { id: "team", title: "Team" },
];

function DataSection(): React.ReactNode {
  const [selectedIndexes, setSelectedIndexes] = useState<number[]>([]);

  const selectedNames = selectedIndexes.map((i) => contacts[i]?.name ?? "?").join(", ");

  return (
    <box orientation="vertical" spacing={Spacing.sm} style={{ vexpand: true, padding: ContentMargin }} testID="data-section">
      <label
        text="Multiple selection, plus draggable column headers. Drag one to reorder."
        cssClasses={["dimmed", "caption"]}
        testID="data-hint"
      />
      <table
        columns={contactColumns}
        rows={contacts.map((c): TableRow => ({ id: c.id, cells: [c.name, c.role, c.team] }))}
        selectionMode="multiple"
        selectedIndexes={selectedIndexes}
        columnsReorderable
        onSelectionChanged={(e) => setSelectedIndexes(e.data.indexes)}
        style={{ vexpand: true }}
        testID="data-table"
      />
      <label
        text={`Selected: ${selectedIndexes.length > 0 ? selectedNames : "(none)"}`}
        cssClasses={["dimmed"]}
        testID="data-selected-readout"
      />
    </box>
  );
}

// --- Overlays ----------------------------------------------------------

type SheetEdge = "top" | "bottom" | "leading" | "trailing";
const sheetEdges: SheetEdge[] = ["top", "bottom", "leading", "trailing"];

function OverlaysSection(): React.ReactNode {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [formName, setFormName] = useState("");
  const [formEmail, setFormEmail] = useState("");
  const [lastDialogAction, setLastDialogAction] = useState("(none yet)");

  const [sheetOpen, setSheetOpen] = useState(false);
  const [sheetEdge, setSheetEdge] = useState<SheetEdge>("bottom");

  function saveDialog(): void {
    setLastDialogAction(`Saved ${formName || "(empty)"} <${formEmail || "(empty)"}>`);
    setDialogOpen(false);
  }

  function cancelDialog(): void {
    setLastDialogAction("Canceled");
    setDialogOpen(false);
  }

  function openSheet(edge: SheetEdge): void {
    setSheetEdge(edge);
    setSheetOpen(true);
  }

  return (
    <scrollview testID="overlays-scroll" style={{ vexpand: true }}>
      <box orientation="vertical" style={{ padding: ContentMargin }}>
        <settingsgroup title="Dialog" description="Arbitrary content in a modal, closed from inside its own form." testID="overlays-dialog-group">
          <row title="Open" testID="overlays-dialog-open-row">
            <button label="Edit Profile…" onClick={() => setDialogOpen(true)} testID="overlays-dialog-open-button" />
          </row>
          <row title="Last action" testID="overlays-dialog-readout-row">
            <label text={lastDialogAction} cssClasses={["dimmed"]} testID="overlays-dialog-readout" />
          </row>
        </settingsgroup>

        <settingsgroup title="Sheet" description="One instance whose edge switches per button, sliding in from each side." testID="overlays-sheet-group">
          <row title="Open from" testID="overlays-sheet-row">
            <box orientation="horizontal" spacing={Spacing.sm} cssClasses={["linked"]}>
              {sheetEdges.map((edge) => (
                <button key={edge} label={edge} onClick={() => openSheet(edge)} testID={`overlays-sheet-open-${edge}`} />
              ))}
            </box>
          </row>
          <row title="State" testID="overlays-sheet-state-row">
            <label text={sheetOpen ? `Open from ${sheetEdge}` : "Closed"} cssClasses={["dimmed"]} testID="overlays-sheet-readout" />
          </row>
        </settingsgroup>

        <dialog
          open={dialogOpen}
          title="Edit Profile"
          contentWidth={380}
          onClosed={() => setDialogOpen(false)}
          testID="overlays-dialog"
        >
          <box orientation="vertical" spacing={Spacing.md}>
            <settingsgroup>
              <row title="Name" testID="overlays-dialog-name-row">
                <textinput text={formName} onChanged={(e) => setFormName(e.text)} testID="overlays-dialog-name-input" />
              </row>
              <row title="Email" testID="overlays-dialog-email-row">
                <textinput text={formEmail} onChanged={(e) => setFormEmail(e.text)} testID="overlays-dialog-email-input" />
              </row>
            </settingsgroup>
            <box orientation="horizontal" spacing={Spacing.sm} style={{ halign: "end" }}>
              <button label="Cancel" onClick={cancelDialog} testID="overlays-dialog-cancel" />
              <button label="Save" prominent onClick={saveDialog} testID="overlays-dialog-save" />
            </box>
          </box>
        </dialog>

        <sheet open={sheetOpen} edge={sheetEdge} size={280} onClosed={() => setSheetOpen(false)} testID="overlays-sheet">
          <box orientation="vertical" spacing={Spacing.sm}>
            <label text={`Sheet from ${sheetEdge}`} cssClasses={["heading"]} testID="overlays-sheet-title" />
            <label text="Slides in from the edge you picked." cssClasses={["dimmed"]} />
            <button label="Close" onClick={() => setSheetOpen(false)} testID="overlays-sheet-close" />
          </box>
        </sheet>
      </box>
    </scrollview>
  );
}

// --- Rich text -----------------------------------------------------------

const richTextMarkdown = `# Rich Text

A read-only, natively parsed **Markdown** view. It renders *italic*, \`inline code\`, and
[links](https://nativedesktop.dev). Clicking a link fires \`linkActivated\` instead of navigating.

## Lists

- Dash bullet
- Second dash bullet

* Star bullet
* Second star bullet

+ Plus bullet
+ Second plus bullet

## Code block

\`\`\`
function greet(name) {
  console.log("hi " + name);
}
\`\`\`

### Not supported

Ordered lists, block quotes, tables, images, and raw HTML sit outside the parsed subset, so they
render as plain text if they appear at all.`;

function RichTextSection(): React.ReactNode {
  const [lastLink, setLastLink] = useState("(none yet)");
  const [selectable, setSelectable] = useState(true);

  return (
    <scrollview testID="richtext-scroll" style={{ vexpand: true }}>
      <box orientation="vertical" spacing={Spacing.sm} style={{ padding: ContentMargin }}>
        <settingsgroup title="RichText" description="Every supported Markdown construct, plus the notable ones that aren't." testID="richtext-group">
          <row title="Selectable" testID="richtext-selectable-row">
            <switch checked={selectable} onToggled={(e) => setSelectable(e.checked)} testID="richtext-selectable-toggle" />
          </row>
        </settingsgroup>
        <richtext
          markdown={richTextMarkdown}
          selectable={selectable}
          onLinkActivated={(e) => {
            setLastLink(e.text);
            void openExternal(e.text);
          }}
          testID="richtext-view"
        />
        <label text={`Last link activated: ${lastLink}`} cssClasses={["dimmed", "caption"]} testID="richtext-link-readout" />
      </box>
    </scrollview>
  );
}

// --- Charts ------------------------------------------------------------

type ChartType = "line" | "area" | "bar" | "pie" | "scatter" | "candlestick";
const chartTypeOptions: ChartType[] = ["line", "area", "bar", "pie", "scatter", "candlestick"];

function describeSelection(chartName: string, e: { data: unknown }): string {
  const d = e.data as { seriesId: string; index: number; x: number; y: number };
  return `${chartName}: "${d.seriesId}" point ${d.index} (x=${d.x}, y=${d.y})`;
}

// Line and area read the same two-series dataset, so the mark difference is
// the only thing that changes between them.
const trendSeries = [
  {
    id: "revenue",
    label: "Revenue",
    points: [
      { x: 1, y: 32 },
      { x: 2, y: 41 },
      { x: 3, y: 38 },
      { x: 4, y: 52 },
      { x: 5, y: 49 },
      { x: 6, y: 61 },
    ],
  },
  {
    id: "cost",
    label: "Cost",
    points: [
      { x: 1, y: 24 },
      { x: 2, y: 27 },
      { x: 3, y: 30 },
      { x: 4, y: 33 },
      { x: 5, y: 31 },
      { x: 6, y: 36 },
    ],
  },
];

const barSeries = [
  {
    id: "downloads",
    label: "Downloads",
    points: [
      { x: 1, y: 120, label: "Q1" },
      { x: 2, y: 150, label: "Q2" },
      { x: 3, y: 98, label: "Q3" },
      { x: 4, y: 210, label: "Q4" },
    ],
  },
];

// A pie reads only its first series; each point's label names the slice.
const pieSeries = [
  {
    id: "browsers",
    label: "Browser share",
    points: [
      { x: 0, y: 48, label: "Chrome" },
      { x: 0, y: 27, label: "Safari" },
      { x: 0, y: 15, label: "Firefox" },
      { x: 0, y: 10, label: "Other" },
    ],
  },
];

const scatterSeries = [
  {
    id: "teamA",
    label: "Team A",
    points: [
      { x: 2, y: 14 },
      { x: 3, y: 18 },
      { x: 4, y: 16 },
      { x: 5, y: 22 },
      { x: 6, y: 19 },
    ],
  },
  {
    id: "teamB",
    label: "Team B",
    points: [
      { x: 2, y: 9 },
      { x: 3, y: 12 },
      { x: 4, y: 15 },
      { x: 5, y: 11 },
      { x: 6, y: 17 },
    ],
  },
];

const candlestickSeries = [
  {
    id: "ndsk",
    label: "NDSK",
    points: [
      { x: 1, y: 104.8, open: 102.5, high: 105.2, low: 101.0, close: 104.8 },
      { x: 2, y: 106.3, open: 104.8, high: 107.1, low: 103.9, close: 106.3 },
      { x: 3, y: 103.9, open: 106.3, high: 106.9, low: 103.2, close: 103.9 },
      { x: 4, y: 101.2, open: 103.9, high: 104.5, low: 100.7, close: 101.2 },
      { x: 5, y: 103.1, open: 101.2, high: 103.8, low: 100.9, close: 103.1 },
    ],
  },
];

// Shared by the Live tab's type switcher: two series, with OHLC fields
// present on every point, so all six types read the same dataset.
const liveSeries = [
  {
    id: "north",
    label: "North",
    points: [
      { x: 1, y: 28, label: "Mon", open: 26, high: 30, low: 24, close: 28 },
      { x: 2, y: 34, label: "Tue", open: 28, high: 36, low: 27, close: 34 },
      { x: 3, y: 31, label: "Wed", open: 34, high: 35, low: 29, close: 31 },
      { x: 4, y: 39, label: "Thu", open: 31, high: 41, low: 30, close: 39 },
      { x: 5, y: 45, label: "Fri", open: 39, high: 47, low: 38, close: 45 },
    ],
  },
  {
    id: "south",
    label: "South",
    points: [
      { x: 1, y: 19, label: "Mon", open: 17, high: 21, low: 16, close: 19 },
      { x: 2, y: 22, label: "Tue", open: 19, high: 24, low: 18, close: 22 },
      { x: 3, y: 26, label: "Wed", open: 22, high: 28, low: 21, close: 26 },
      { x: 4, y: 24, label: "Thu", open: 26, high: 27, low: 22, close: 24 },
      { x: 5, y: 30, label: "Fri", open: 24, high: 32, low: 23, close: 30 },
    ],
  },
];

function ChartsSection(): React.ReactNode {
  const [tabIndex, setTabIndex] = useState(0);
  const [typeIndex, setTypeIndex] = useState(0);
  const [showLegend, setShowLegend] = useState(true);
  const [showGrid, setShowGrid] = useState(true);
  const [animated, setAnimated] = useState(true);
  const [lastSelection, setLastSelection] = useState("(none yet)");

  const liveType: ChartType = chartTypeOptions[typeIndex] ?? "line";

  return (
    <box orientation="vertical" spacing={Spacing.sm} style={{ vexpand: true, padding: ContentMargin }} testID="charts-section">
      <tabview
        selectedIndex={tabIndex}
        onSelectionChanged={(e) => setTabIndex(e.index)}
        style={{ vexpand: true }}
        testID="charts-tabs"
      >
        <box tabLabel="Types" orientation="vertical" spacing={Spacing.sm} style={{ vexpand: true }}>
          <label
            text="All six chart types. Line and area share the same two-series dataset; click a point."
            cssClasses={["dimmed", "caption"]}
            testID="charts-types-hint"
          />
          <grid style={{ vexpand: true }} testID="charts-types-grid">
            <chart
              type="line"
              series={trendSeries}
              xLabel="Month"
              yLabel="Amount"
              onPointSelected={(e) => setLastSelection(describeSelection("Line", e))}
              gridRow={0}
              gridColumn={0}
              style={{ vexpand: true, hexpand: true }}
              testID="charts-line"
            />
            <chart
              type="area"
              series={trendSeries}
              xLabel="Month"
              yLabel="Amount"
              onPointSelected={(e) => setLastSelection(describeSelection("Area", e))}
              gridRow={0}
              gridColumn={1}
              style={{ vexpand: true, hexpand: true }}
              testID="charts-area"
            />
            <chart
              type="bar"
              series={barSeries}
              xLabel="Quarter"
              yLabel="Downloads"
              onPointSelected={(e) => setLastSelection(describeSelection("Bar", e))}
              gridRow={1}
              gridColumn={0}
              style={{ vexpand: true, hexpand: true }}
              testID="charts-bar"
            />
            <chart
              type="pie"
              series={pieSeries}
              onPointSelected={(e) => setLastSelection(describeSelection("Pie", e))}
              gridRow={1}
              gridColumn={1}
              style={{ vexpand: true, hexpand: true }}
              testID="charts-pie"
            />
            <chart
              type="scatter"
              series={scatterSeries}
              xLabel="Week"
              yLabel="Output"
              onPointSelected={(e) => setLastSelection(describeSelection("Scatter", e))}
              gridRow={2}
              gridColumn={0}
              style={{ vexpand: true, hexpand: true }}
              testID="charts-scatter"
            />
            <chart
              type="candlestick"
              series={candlestickSeries}
              xLabel="Day"
              yLabel="Price"
              onPointSelected={(e) => setLastSelection(describeSelection("Candlestick", e))}
              gridRow={2}
              gridColumn={1}
              style={{ vexpand: true, hexpand: true }}
              testID="charts-candlestick"
            />
          </grid>
        </box>

        <box tabLabel="Live" orientation="vertical" spacing={Spacing.sm} style={{ vexpand: true }}>
          <settingsgroup title="Live chart" description="One dataset, switching type and rendering options." testID="charts-live-group">
            <row title="Type" testID="charts-live-type-row">
              <select
                options={chartTypeOptions}
                selectedIndex={typeIndex}
                onSelectionChanged={(e) => setTypeIndex(e.index)}
                testID="charts-live-type-select"
              />
            </row>
            <row title="Legend" testID="charts-live-legend-row">
              <switch checked={showLegend} onToggled={(e) => setShowLegend(e.checked)} testID="charts-live-legend-toggle" />
            </row>
            <row title="Grid" testID="charts-live-grid-row">
              <switch checked={showGrid} onToggled={(e) => setShowGrid(e.checked)} testID="charts-live-grid-toggle" />
            </row>
            <row title="Animated" testID="charts-live-animated-row">
              <switch checked={animated} onToggled={(e) => setAnimated(e.checked)} testID="charts-live-animated-toggle" />
            </row>
          </settingsgroup>
          <chart
            type={liveType}
            series={liveSeries}
            xLabel="Day"
            yLabel="Value"
            showLegend={showLegend}
            showGrid={showGrid}
            animated={animated}
            onPointSelected={(e) => setLastSelection(describeSelection("Live", e))}
            style={{ vexpand: true }}
            testID="charts-live-chart"
          />
        </box>
      </tabview>
      <label text={`Last selected: ${lastSelection}`} cssClasses={["dimmed"]} testID="charts-selection-readout" />
    </box>
  );
}

// --- Progress ------------------------------------------------------------

const progressFractions = [0, 0.25, 0.5, 0.75, 1];

function ProgressSection(): React.ReactNode {
  const [fraction, setFraction] = useState(0.35);
  const [spinning, setSpinning] = useState(true);

  return (
    <scrollview testID="progress-scroll" style={{ vexpand: true }}>
      <box orientation="vertical" style={{ padding: ContentMargin }}>
        <settingsgroup title="Live fraction" description="One slider drives a ProgressBar and a ProgressCircle together." testID="progress-live-group">
          <row title="Fraction" testID="progress-live-row">
            <box orientation="horizontal" spacing={Spacing.sm} style={{ hexpand: true }}>
              <slider min={0} max={1} step={0.01} value={fraction} onValueChanged={(e) => setFraction(e.value)} style={{ hexpand: true }} testID="progress-live-slider" />
              <label text={`${Math.round(fraction * 100)}%`} cssClasses={["dimmed", "numeric"]} testID="progress-live-readout" />
            </box>
          </row>
          <row title="ProgressBar" testID="progress-live-bar-row">
            <progressbar fraction={fraction} style={{ hexpand: true }} testID="progress-live-bar" />
          </row>
          <row title="ProgressCircle" testID="progress-live-circle-row">
            <progresscircle fraction={fraction} showLabel testID="progress-live-circle" />
          </row>
        </settingsgroup>

        <settingsgroup title="ProgressCircle" description="Fixed fractions, with and without the label." testID="progress-circle-group">
          <row title="Label on" testID="progress-circle-labeled-row">
            <box orientation="horizontal" spacing={Spacing.md}>
              {progressFractions.map((f) => (
                <progresscircle key={f} fraction={f} showLabel testID={`progress-circle-labeled-${Math.round(f * 100)}`} />
              ))}
            </box>
          </row>
          <row title="Label off" testID="progress-circle-unlabeled-row">
            <box orientation="horizontal" spacing={Spacing.md}>
              {progressFractions.map((f) => (
                <progresscircle key={f} fraction={f} testID={`progress-circle-unlabeled-${Math.round(f * 100)}`} />
              ))}
            </box>
          </row>
        </settingsgroup>

        <settingsgroup title="Spinner" description="Indeterminate activity, toggled on and off." testID="progress-spinner-group">
          <row title="Spinning" testID="progress-spinner-row">
            <box orientation="horizontal" spacing={Spacing.sm}>
              <spinner spinning={spinning} testID="progress-spinner" />
              <switch checked={spinning} onToggled={(e) => setSpinning(e.checked)} testID="progress-spinner-toggle" />
            </box>
          </row>
        </settingsgroup>
      </box>
    </scrollview>
  );
}

// --- Loading -------------------------------------------------------------

interface LoadingRow {
  id: string;
  name: string;
  detail: string;
}

const loadingRows: LoadingRow[] = [
  { id: "r1", name: "Ada Lovelace", detail: "Engineering · Online" },
  { id: "r2", name: "Grace Hopper", detail: "Research · Away" },
  { id: "r3", name: "Alan Turing", detail: "Security · Offline" },
];

function LoadingSection(): React.ReactNode {
  const [loaded, setLoaded] = useState(false);

  return (
    <scrollview testID="loading-scroll" style={{ vexpand: true }}>
      <box orientation="vertical" style={{ padding: ContentMargin }}>
        <settingsgroup title="List placeholder" description="Skeletons respect the OS reduce-motion setting." testID="loading-group">
          <row title="Loaded" testID="loading-toggle-row">
            <switch checked={loaded} onToggled={(e) => setLoaded(e.checked)} testID="loading-toggle" />
          </row>
        </settingsgroup>

        <box orientation="vertical" testID="loading-list">
          {loadingRows.map((r, i) => (
            <box key={r.id} orientation="vertical" testID={`loading-row-${r.id}`}>
              <box orientation="horizontal" spacing={Spacing.md} style={{ padding: Spacing.sm }}>
                {loaded ? (
                  <avatar text={r.name} size={40} testID={`loading-row-${r.id}-avatar`} />
                ) : (
                  <skeleton width={40} height={40} radius={20} testID={`loading-row-${r.id}-avatar-skeleton`} />
                )}
                <box orientation="vertical" spacing={Spacing.xs} style={{ hexpand: true, valign: "center" }}>
                  {loaded ? (
                    <label text={r.name} cssClasses={["heading"]} testID={`loading-row-${r.id}-name`} />
                  ) : (
                    <skeleton width={140} height={14} testID={`loading-row-${r.id}-name-skeleton`} />
                  )}
                  {loaded ? (
                    <label text={r.detail} cssClasses={["dimmed", "caption"]} testID={`loading-row-${r.id}-detail`} />
                  ) : (
                    <skeleton width={90} height={10} testID={`loading-row-${r.id}-detail-skeleton`} />
                  )}
                </box>
              </box>
              {i < loadingRows.length - 1 && <separator orientation="horizontal" />}
            </box>
          ))}
        </box>
      </box>
    </scrollview>
  );
}

// --- Dock ----------------------------------------------------------------

interface DockTabData {
  body: string;
}

function seedDockModel(): DockModel<DockTabData> {
  return seedDock<DockTabData>([
    [
      { id: "explorer", title: "Explorer", data: { body: "File tree goes here." } },
      { id: "search", title: "Search", data: { body: "Search results go here." } },
    ],
    [{ id: "output", title: "Output", data: { body: "Build output goes here." } }],
    [{ id: "terminal", title: "Terminal", data: { body: "Shell session goes here." } }],
  ]);
}

// seedDock assigns panel ids "1"/"2"/"3" in group order (seedPanes' default
// id, String(i+1)): "1" is Explorer/Search, "2" is Output, "3" is Terminal.
const dockZones: DockZone[] = ["left", "right", "top", "bottom", "center"];

function DockSection(): React.ReactNode {
  const dock = useDock<DockTabData>(seedDockModel);
  const [nextTabId, setNextTabId] = useState(1);

  return (
    <box orientation="vertical" style={{ vexpand: true, padding: ContentMargin }} testID="dock-section">
      <label text="Dock panel 2 (Output) relative to panel 1 (Explorer/Search)" cssClasses={["dimmed", "caption"]} testID="dock-zone-label" />
      <box orientation="horizontal" spacing={Spacing.xs} cssClasses={["linked"]} testID="dock-zone-row">
        {dockZones.map((zone) => (
          <button key={zone} label={zone} onClick={() => dock.dock("2", "1", zone)} testID={`dock-zone-${zone}`} />
        ))}
      </box>
      <box orientation="horizontal" spacing={Spacing.sm} testID="dock-action-row">
        <button label="Undock Output" onClick={() => dock.undockTab("output", "right")} testID="dock-undock-output" />
        <button label="Close Terminal" onClick={() => dock.closeTab("terminal")} testID="dock-close-terminal" />
        <button
          label="Add tab to panel 1"
          onClick={() => {
            dock.addTab("1", { id: `extra-${nextTabId}`, title: `Extra ${nextTabId}`, data: { body: "Added at runtime." } });
            setNextTabId((n) => n + 1);
          }}
          testID="dock-add-tab"
        />
      </box>
      <separator orientation="horizontal" />
      <box style={{ vexpand: true }} testID="dock-canvas">
        <DockView
          model={dock.model}
          onChange={dock.setModel}
          testID="parity-dock"
          renderTab={({ tab }) => (
            <box orientation="vertical" style={{ padding: Spacing.md, vexpand: true }} testID={`dock-tab-body-${tab.id}`}>
              <label text={tab.data.body} cssClasses={["dimmed"]} />
            </box>
          )}
        />
      </box>
    </box>
  );
}

// --- Tiles -----------------------------------------------------------------

interface TileData {
  label: string;
}

function seedTilesModel(): TileModel<TileData> {
  return seedTiles<TileData>(
    [
      { id: "a", x: 0, y: 0, w: 2, h: 2, data: { label: "Widget A" } },
      { id: "b", x: 2, y: 0, w: 2, h: 1, data: { label: "Widget B" } },
      { id: "c", x: 0, y: 2, w: 1, h: 1, data: { label: "Widget C" } },
    ],
    4,
  );
}

function TilesSection(): React.ReactNode {
  const tiles = useTiles<TileData>(seedTilesModel);
  const [nextTileId, setNextTileId] = useState(1);

  return (
    <box orientation="vertical" style={{ vexpand: true, padding: ContentMargin }} testID="tiles-section">
      <box orientation="horizontal" spacing={Spacing.sm} testID="tiles-action-row">
        <button
          label="Place tile at (0,0)"
          onClick={() => {
            tiles.place({ id: `extra-${nextTileId}`, x: 0, y: 0, w: 1, h: 1, data: { label: `Extra ${nextTileId}` } });
            setNextTileId((n) => n + 1);
          }}
          testID="tiles-place"
        />
        <button label="Move B to (0,0)" onClick={() => tiles.move("b", 0, 0)} testID="tiles-move-b" />
        <button label="Resize A to 3x3" onClick={() => tiles.resize("a", 3, 3)} testID="tiles-resize-a" />
        <button label="Raise C" onClick={() => tiles.raise("c")} testID="tiles-raise-c" />
        <button label="Remove B" onClick={() => tiles.remove("b")} testID="tiles-remove-b" />
      </box>
      <separator orientation="horizontal" />
      <box style={{ vexpand: true }} testID="tiles-canvas">
        <TilesView
          model={tiles.model}
          testID="parity-tiles"
          renderTile={({ tile, raised }) => (
            <box
              orientation="vertical"
              spacing={Spacing.xs}
              style={{ padding: Spacing.md, vexpand: true, hexpand: true }}
              cssClasses={raised ? ["card", "accent"] : ["card"]}
              testID={`tiles-tile-${tile.id}`}
            >
              <label text={tile.data.label} cssClasses={["heading"]} />
              <label text={`${tile.w}x${tile.h} @ (${tile.x},${tile.y})`} cssClasses={["dimmed", "caption"]} />
            </box>
          )}
        />
      </box>
    </box>
  );
}

// --- Drag and drop -----------------------------------------------------

// draggable/dragPayload/dropTarget and dragStarted/dragEnded/dragOver/dropped
// are universal props and events, present on every widget, not a dedicated
// drag-and-drop widget. This kanban board is a box of dropTarget columns
// holding draggable cards.
type KanbanColumnId = "todo" | "doing" | "done";

interface KanbanCard {
  id: string;
  title: string;
  columnId: KanbanColumnId;
}

const kanbanColumns: { id: KanbanColumnId; title: string }[] = [
  { id: "todo", title: "To Do" },
  { id: "doing", title: "Doing" },
  { id: "done", title: "Done" },
];

const initialKanbanCards: KanbanCard[] = [
  { id: "card-1", title: "Design the board", columnId: "todo" },
  { id: "card-2", title: "Wire dragOver", columnId: "todo" },
  { id: "card-3", title: "Ship the docs", columnId: "doing" },
  { id: "card-4", title: "Review parity gallery", columnId: "done" },
];

function DragDropSection(): React.ReactNode {
  const [cards, setCards] = useState<KanbanCard[]>(initialKanbanCards);
  // dragPoint doubles as the hover highlight: it is set on every dragOver and
  // cleared on drop or on dragEnded, so a column's accent state and its x/y
  // readout always agree.
  const [dragPoint, setDragPoint] = useState<{ columnId: KanbanColumnId; x: number; y: number } | null>(null);
  const [lastDropped, setLastDropped] = useState("(none yet)");

  function moveCard(cardId: string, columnId: KanbanColumnId): void {
    setCards((prev) => prev.map((c) => (c.id === cardId ? { ...c, columnId } : c)));
  }

  const hoveredColumn = dragPoint ? kanbanColumns.find((c) => c.id === dragPoint.columnId) : undefined;

  return (
    <scrollview testID="dragdrop-scroll" style={{ vexpand: true }}>
      <box orientation="vertical" spacing={Spacing.sm} style={{ padding: ContentMargin }}>
        <settingsgroup
          title="Drag and drop"
          description="draggable and dropTarget are universal props on every widget. Drag a card between columns."
          testID="dragdrop-group"
        >
          <row title="Hovered column" testID="dragdrop-hover-row">
            <label text={hoveredColumn ? hoveredColumn.title : "(none)"} cssClasses={["dimmed"]} testID="dragdrop-hover-readout" />
          </row>
          <row title="dragOver x, y" subtitle="Target widget's own coordinate space, top-left origin" testID="dragdrop-point-row">
            <label
              text={dragPoint ? `x=${Math.round(dragPoint.x)}, y=${Math.round(dragPoint.y)}` : "(not dragging)"}
              cssClasses={["dimmed", "numeric"]}
              testID="dragdrop-point-readout"
            />
          </row>
          <row title="Last dropped" testID="dragdrop-last-row">
            <label text={lastDropped} cssClasses={["dimmed"]} testID="dragdrop-last-readout" />
          </row>
        </settingsgroup>

        <box orientation="horizontal" spacing={Spacing.md} style={{ vexpand: true }} testID="dragdrop-board">
          {kanbanColumns.map((column) => {
            const columnCards = cards.filter((c) => c.columnId === column.id);
            const hovered = dragPoint?.columnId === column.id;
            return (
              <box
                key={column.id}
                orientation="vertical"
                spacing={Spacing.xs}
                style={{ padding: Spacing.sm, hexpand: true, vexpand: true }}
                cssClasses={hovered ? ["card", "accent"] : ["card"]}
                dropTarget
                onDragOver={(e) => setDragPoint({ columnId: column.id, x: e.data.x, y: e.data.y })}
                onDropped={(e) => {
                  moveCard(e.text, column.id);
                  setLastDropped(`${e.text} to ${column.title}`);
                  setDragPoint(null);
                }}
                testID={`dragdrop-column-${column.id}`}
              >
                <label text={column.title} cssClasses={["heading"]} testID={`dragdrop-column-${column.id}-title`} />
                {columnCards.map((card) => (
                  <box
                    key={card.id}
                    orientation="vertical"
                    style={{ padding: Spacing.sm }}
                    cssClasses={["card", "activatable"]}
                    draggable
                    dragPayload={card.id}
                    onDragEnded={() => setDragPoint(null)}
                    testID={`dragdrop-card-${card.id}`}
                  >
                    <label text={card.title} testID={`dragdrop-card-${card.id}-title`} />
                  </box>
                ))}
                {columnCards.length === 0 && (
                  <label text="Drop here" cssClasses={["dimmed", "caption"]} testID={`dragdrop-column-${column.id}-empty`} />
                )}
              </box>
            );
          })}
        </box>
      </box>
    </scrollview>
  );
}

// --- Code editor ---------------------------------------------------------

// CodeDiagnostic isn't re-exported from @nativedesktop/react yet, so this
// array is checked structurally against <codeeditor>'s diagnostics prop
// rather than against an imported type, the same situation charts.md
// documents for ChartSeries/ChartPoint.
const codeEditorLanguages = ["typescript", "javascript", "python", "rust"];

const codeEditorSample = `function greet(name: string): string {
  return "Hello, " + nam;
}

const total = 1 + undefinedVar;
`;

const codeEditorDiagnostics = [
  { line: 2, column: 22, severity: "error", message: "Cannot find name 'nam'. Did you mean 'name'?" },
  { line: 5, column: 19, severity: "warning", message: "'undefinedVar' is not defined." },
];

function CodeEditorSection(): React.ReactNode {
  const [code, setCode] = useState(codeEditorSample);
  const [languageIndex, setLanguageIndex] = useState(0);
  const [showLineNumbers, setShowLineNumbers] = useState(true);
  const [readOnly, setReadOnly] = useState(false);
  const [cursor, setCursor] = useState("(none yet)");
  const [lastDiagnostic, setLastDiagnostic] = useState("(none yet)");

  return (
    <scrollview testID="codeeditor-scroll" style={{ vexpand: true }}>
      <box orientation="vertical" spacing={Spacing.sm} style={{ padding: ContentMargin }}>
        <settingsgroup
          title="Code Editor"
          description="Two diagnostics are pinned to the sample below. Click a squiggle to read its message."
          testID="codeeditor-group"
        >
          <row title="Language" testID="codeeditor-language-row">
            <select
              options={codeEditorLanguages}
              selectedIndex={languageIndex}
              onSelectionChanged={(e) => setLanguageIndex(e.index)}
              testID="codeeditor-language-select"
            />
          </row>
          <row title="Line numbers" testID="codeeditor-linenumbers-row">
            <switch checked={showLineNumbers} onToggled={(e) => setShowLineNumbers(e.checked)} testID="codeeditor-linenumbers-toggle" />
          </row>
          <row title="Read only" testID="codeeditor-readonly-row">
            <switch checked={readOnly} onToggled={(e) => setReadOnly(e.checked)} testID="codeeditor-readonly-toggle" />
          </row>
          <row title="Cursor" testID="codeeditor-cursor-row">
            <label text={cursor} cssClasses={["dimmed", "numeric"]} testID="codeeditor-cursor-readout" />
          </row>
          <row title="Last diagnostic clicked" testID="codeeditor-diagnostic-row">
            <label text={lastDiagnostic} cssClasses={["dimmed"]} testID="codeeditor-diagnostic-readout" />
          </row>
        </settingsgroup>

        <codeeditor
          text={code}
          language={codeEditorLanguages[languageIndex]}
          showLineNumbers={showLineNumbers}
          readOnly={readOnly}
          diagnostics={codeEditorDiagnostics}
          onChange={(e) => setCode(e.text)}
          onCursorMoved={(e) => {
            const { line, column } = e.data as { line: number; column: number };
            setCursor(`Line ${line}, Column ${column}`);
          }}
          onDiagnosticClicked={(e) => {
            const d = e.data as { line: number; column: number; severity: string; message: string };
            setLastDiagnostic(`${d.severity} at line ${d.line}, column ${d.column}: ${d.message}`);
          }}
          style={{ vexpand: true }}
          testID="codeeditor-widget"
        />
      </box>
    </scrollview>
  );
}

// --- App ---------------------------------------------------------------

function renderSection(section: SectionId): React.ReactNode {
  switch (section) {
    case "display":
      return <DisplaySection />;
    case "input":
      return <InputSection />;
    case "navigation":
      return <NavigationSection />;
    case "composition":
      return <CompositionSection />;
    case "data":
      return <DataSection />;
    case "overlays":
      return <OverlaysSection />;
    case "richtext":
      return <RichTextSection />;
    case "charts":
      return <ChartsSection />;
    case "progress":
      return <ProgressSection />;
    case "loading":
      return <LoadingSection />;
    case "dock":
      return <DockSection />;
    case "tiles":
      return <TilesSection />;
    case "dragdrop":
      return <DragDropSection />;
    case "codeeditor":
      return <CodeEditorSection />;
  }
}

function App(): React.ReactNode {
  const [section, setSection] = useState<SectionId>("display");

  return (
    <window title="Parity Gallery" defaultWidth={1200} defaultHeight={780}>
      <splitview sidebarWidth={0.22} testID="parity-split">
        <toolbarview slot="sidebar" testID="sidebar-toolbar">
          <headerbar title="Parity Gallery" testID="sidebar-header" />
          <sourcetree
            testID="parity-nav"
            nodes={navNodes}
            selectedId={section}
            onSelectionChanged={(e) => {
              const { nodeId } = e.data as { nodeId: string | null };
              if (nodeId) setSection(nodeId as SectionId);
            }}
            style={{ vexpand: true }}
          />
        </toolbarview>
        <toolbarview slot="content" testID="content-toolbar">
          <headerbar title={sectionTitles[section]} testID="content-header" />
          {renderSection(section)}
        </toolbarview>
      </splitview>
    </window>
  );
}

await render(<App />);
