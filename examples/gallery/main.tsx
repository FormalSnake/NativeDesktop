import {
  render,
  useMemo,
  useRef,
  useState,
  Platform,
  showAlert,
  openFile,
  saveFile,
  showAbout,
  onAlertResult,
  onOpenFileResult,
  onSaveFileResult,
  showToast,
  onToastButtonClicked,
  onToastDismissed,
} from "@nativedesktop/react";
import type { NdNodeRef, SourceTreeAction, SourceTreeNode, TableColumn, TableRow, TreeNode } from "@nativedesktop/react";

// Widget gallery: every widget here has live controlled state + testIDs,
// driven headlessly by scripts/m5b-drive.ts and scripts/m5c-drive.ts over
// the automation socket. The original Form/Grid/Styled/List/SourceList tabs
// are untouched (same testIDs, same structure) — the drive scripts walk the
// automation tree by testID, so new tabs and the ToastOverlay wrapper below
// don't disturb them.
//
// The newer widgets get one showcase tab each, grouped by kind (Controls /
// Pickers / Popovers & Menus / Status & Banner / Toasts / Table / Tree /
// Dialogs / Video / macOS). Every control is wired to real state (no dead
// props), like the rest of this file.

interface Employee {
  id: string;
  name: string;
  role: string;
  department: string;
  years: number;
  status: string;
}

const initialEmployees: Employee[] = [
  { id: "1", name: "Ada Lovelace", role: "Engineer", department: "Platform", years: 5, status: "Active" },
  { id: "2", name: "Grace Hopper", role: "Engineer", department: "Compiler", years: 12, status: "Active" },
  { id: "3", name: "Alan Turing", role: "Researcher", department: "Theory", years: 8, status: "Active" },
  { id: "4", name: "Margaret Hamilton", role: "Lead", department: "Flight Software", years: 10, status: "Active" },
  { id: "5", name: "Katherine Johnson", role: "Analyst", department: "Trajectory", years: 15, status: "Active" },
  { id: "6", name: "Dennis Ritchie", role: "Engineer", department: "Systems", years: 9, status: "On Leave" },
  { id: "7", name: "Barbara Liskov", role: "Researcher", department: "Languages", years: 7, status: "Active" },
  { id: "8", name: "Donald Knuth", role: "Researcher", department: "Algorithms", years: 20, status: "Active" },
  { id: "9", name: "Radia Perlman", role: "Engineer", department: "Networking", years: 6, status: "Active" },
  { id: "10", name: "Tim Berners-Lee", role: "Architect", department: "Web", years: 11, status: "Active" },
];

const tableColumns: TableColumn[] = [
  { id: "name", title: "Name" },
  { id: "role", title: "Role" },
  { id: "department", title: "Department", width: 160 },
  { id: "years", title: "Years" },
  { id: "status", title: "Status" },
];

// Flat id/parentId tree — TreeView's data model. `expanded` is controlled
// state (tracked separately below) so a re-render elsewhere in the app can
// never silently collapse a branch the user opened.
const treeNodeMeta: Omit<TreeNode, "expanded">[] = [
  { id: "fruits", title: "Fruits", hasChildren: true },
  { id: "apple", parentId: "fruits", title: "Apple", hasChildren: false },
  { id: "banana", parentId: "fruits", title: "Banana", hasChildren: false },
  { id: "veggies", title: "Vegetables", hasChildren: true },
  { id: "root-veg", parentId: "veggies", title: "Root Vegetables", hasChildren: true },
  { id: "carrot", parentId: "root-veg", title: "Carrot", hasChildren: false },
  { id: "potato", parentId: "root-veg", title: "Potato", hasChildren: false },
  { id: "leafy", parentId: "veggies", title: "Leafy Greens", hasChildren: false },
];

function treeNodeTitle(nodeId: string | null): string {
  if (!nodeId) return "(none)";
  return treeNodeMeta.find((n) => n.id === nodeId)?.title ?? nodeId;
}

const sampleVideoPath = `${import.meta.dir}/assets/sample.mp4`;

// SourceTree: sections + a 3-level host -> project -> run hierarchy with
// captions, badges, and two trailing actions. Expansion is app-controlled
// (the expanded set below), exactly like the TreeView tab.
const sourceTreeActions: SourceTreeAction[] = [
  { id: "new-run", iconName: "list-add-symbolic", label: "New Run" },
  { id: "close-run", iconName: "window-close-symbolic", tooltip: "Close run", destructive: true },
];

const sourceTreeMeta: Omit<SourceTreeNode, "expanded">[] = [
  { id: "sec-hosts", title: "Hosts", section: true, hasChildren: true, testID: "st-sec-hosts" },
  { id: "host-mac", parentId: "sec-hosts", title: "macbook", caption: "connected", iconName: "computer-symbolic",
    captionIconName: "network-transmit-receive-symbolic", hasChildren: true, testID: "st-host-mac" },
  { id: "proj-nd", parentId: "host-mac", title: "NativeDesktop", hasChildren: true,
    actionIds: ["new-run"], testID: "st-proj-nd" },
  { id: "run-1", parentId: "proj-nd", title: "fix sidebar", caption: "running · 2m", badge: "3",
    actionIds: ["close-run"], testID: "st-run-1" },
  { id: "run-2", parentId: "proj-nd", title: "docs pass", caption: "idle", testID: "st-run-2" },
  { id: "sec-settled", title: "Settled", section: true, hasChildren: true, testID: "st-sec-settled" },
  { id: "run-old", parentId: "sec-settled", title: "old run", caption: "settled yesterday", testID: "st-run-old" },
];

function App(): React.ReactNode {
  const [name, setName] = useState("");
  const [notes, setNotes] = useState("");
  const [agreed, setAgreed] = useState(false);
  const [size, setSize] = useState("small");
  const [fruitIndex, setFruitIndex] = useState(0);
  const [volume, setVolume] = useState(25);
  const [submitted, setSubmitted] = useState(false);
  const fruits = ["apple", "banana", "cherry"];
  const rows = useMemo(() => Array.from({ length: 100_000 }, (_, i) => `Item ${i}`), []);
  const [selectedRow, setSelectedRow] = useState(0);
  const [activatedRow, setActivatedRow] = useState(-1);
  const sourceItems = [
    { title: "Inbox", badge: "3" },
    { title: "Starred", iconName: "starred-symbolic" },
    { title: "Sent" },
    { title: "Archive" },
  ];
  const [sourceSelected, setSourceSelected] = useState(0);

  // Window ref: showAlert/openFile/saveFile/showAbout are commands scoped to
  // this <window> node, correlated to their *Result events by the window's
  // own wire id (see packages/react/src/dialogs.ts's header comment).
  const winRef = useRef<NdNodeRef<"window">>(null);
  const toastRef = useRef<NdNodeRef<"toastoverlay">>(null);

  // --- Controls tab ---
  const [bold, setBold] = useState(false);
  const sizeOptions = ["Small", "Medium", "Large"];
  const sizePreviewPt = [12, 16, 24];
  const [sizeSegmentIndex, setSizeSegmentIndex] = useState(1);
  const [seats, setSeats] = useState(4);
  const [lastLinkActivated, setLastLinkActivated] = useState("");

  // --- Pickers tab ---
  const [color, setColor] = useState("#3366cc");
  const [pickedDate, setPickedDate] = useState("");
  const [fontDesc, setFontDesc] = useState("Sans 12");
  const [levelValue, setLevelValue] = useState(0.4);

  // --- Popovers & Menus tab ---
  const [popoverOpen, setPopoverOpen] = useState(false);
  // The popover below anchors here rather than on its own tree parent.
  const popoverTrigger = useRef<NdNodeRef<"button">>(null);
  const [lastMenuAction, setLastMenuAction] = useState("");
  const [expanderOpen, setExpanderOpen] = useState(false);
  const [expanderChecked, setExpanderChecked] = useState(false);

  // --- Status & Banner tab ---
  const [bannerRevealed, setBannerRevealed] = useState(true);
  const [statusActionCount, setStatusActionCount] = useState(0);

  // --- Toasts tab ---
  const [lastToastResult, setLastToastResult] = useState("");

  // --- Table tab ---
  const [employees, setEmployees] = useState(initialEmployees);
  const [selectedEmployeeIndex, setSelectedEmployeeIndex] = useState(-1);
  const [activatedEmployeeIndex, setActivatedEmployeeIndex] = useState(-1);
  const [lastSort, setLastSort] = useState("");

  function handleSortChanged(e: { data: unknown }): void {
    const { columnId, direction } = e.data as { columnId: string; direction: "ascending" | "descending" };
    const key = columnId as keyof Employee;
    setEmployees((prev) => {
      const sorted = [...prev].sort((a, b) => {
        const av = a[key];
        const bv = b[key];
        const cmp = typeof av === "number" && typeof bv === "number" ? av - bv : String(av).localeCompare(String(bv));
        return direction === "ascending" ? cmp : -cmp;
      });
      return sorted;
    });
    setLastSort(`${columnId} ${direction}`);
  }

  // --- Tree tab ---
  const [treeExpanded, setTreeExpanded] = useState<Set<string>>(new Set(["fruits"]));
  const [selectedTreeNodeId, setSelectedTreeNodeId] = useState<string | null>(null);
  const [activatedTreeNodeId, setActivatedTreeNodeId] = useState<string | null>(null);
  const treeNodes: TreeNode[] = treeNodeMeta.map((n) => ({ ...n, expanded: treeExpanded.has(n.id) }));

  // --- SourceTree tab ---
  const [stExpanded, setStExpanded] = useState<Set<string>>(new Set(["sec-hosts", "host-mac", "proj-nd"]));
  const [stSelectedId, setStSelectedId] = useState("");
  const [stLastAction, setStLastAction] = useState("");
  const [stLastActivated, setStLastActivated] = useState("");
  const [stLastExpandEvent, setStLastExpandEvent] = useState("");
  const stNodes: SourceTreeNode[] = sourceTreeMeta.map((n) => ({ ...n, expanded: stExpanded.has(n.id) }));

  // --- Dialogs tab ---
  const [alertResultText, setAlertResultText] = useState("(none yet)");
  const [openFileResultText, setOpenFileResultText] = useState("(none yet)");
  const [saveFileResultText, setSaveFileResultText] = useState("(none yet)");

  async function handleShowAlert(): Promise<void> {
    if (!winRef.current) return;
    const result = await showAlert(winRef.current, {
      title: "Delete this item?",
      body: "This action cannot be undone.",
      buttons: [
        { id: "cancel", label: "Cancel" },
        { id: "delete", label: "Delete", style: "destructive" },
      ],
    });
    setAlertResultText(result.buttonId);
  }

  async function handleOpenFile(): Promise<void> {
    if (!winRef.current) return;
    const result = await openFile(winRef.current, {
      filters: [{ name: "Text", extensions: ["txt", "md"] }],
    });
    setOpenFileResultText(result.canceled ? "canceled" : result.paths.join(", "));
  }

  async function handleSaveFile(): Promise<void> {
    if (!winRef.current) return;
    const result = await saveFile(winRef.current, { suggestedName: "export.json" });
    setSaveFileResultText(result.canceled ? "canceled" : (result.path ?? "(null)"));
  }

  function handleShowAbout(): void {
    if (!winRef.current) return;
    showAbout(winRef.current, {
      appName: "NativeDesktop Gallery",
      version: "0.1.0",
      developer: "NativeDesktop",
      website: "https://nativedesktop.dev",
    });
  }

  // --- Video tab ---
  const [videoSrc, setVideoSrc] = useState(sampleVideoPath);

  async function handleChooseVideo(): Promise<void> {
    if (!winRef.current) return;
    const result = await openFile(winRef.current, {
      filters: [{ name: "Video", extensions: ["mp4", "webm", "mov", "ogv"] }],
    });
    if (!result.canceled && result.paths[0]) setVideoSrc(result.paths[0]);
  }

  return (
    <window
      ref={winRef}
      title="NativeDesktop Gallery"
      defaultWidth={1000}
      defaultHeight={720}
      onAlertResult={(e) => onAlertResult(winRef.current!, e)}
      onOpenFileResult={(e) => onOpenFileResult(winRef.current!, e)}
      onSaveFileResult={(e) => onSaveFileResult(winRef.current!, e)}
    >
      {/* ToastOverlay wraps the whole tree (childModel: single) so
          showToast() can float a toast above every tab, not just the one
          that triggered it. */}
      <toastoverlay
        ref={toastRef}
        testID="gallery-toast-overlay"
        onToastButtonClicked={onToastButtonClicked}
        onToastDismissed={onToastDismissed}
      >
        <box orientation="vertical" spacing={8}>
          <tabview testID="gallery-tabs">
            <box tabLabel="Form" orientation="vertical" spacing={6}>
              <textinput
                testID="name-input"
                text={name}
                placeholder="Your name"
                onChanged={(e) => setName(e.text)}
                onActivate={() => setSubmitted(true)}
              />
              <label testID="echo-label" text={`Echo: ${name}`} />
              <label testID="submit-label" text={`Submitted: ${submitted ? "yes" : "no"}`} />
              <separator orientation="horizontal" />
              <textarea testID="notes-area" text={notes} onChanged={(e) => setNotes(e.text)} />
              <label testID="notes-label" text={`Notes: ${notes}`} />
              <checkbox testID="agree-check" label="I agree" checked={agreed} onToggled={(e) => setAgreed(e.checked)} />
              <label testID="agree-label" text={`Agreed: ${agreed ? "yes" : "no"}`} />
              <radio testID="size-small" group="size" label="Small" checked={size === "small"}
                onToggled={(e) => { if (e.checked) setSize("small"); }} />
              <radio testID="size-large" group="size" label="Large" checked={size === "large"}
                onToggled={(e) => { if (e.checked) setSize("large"); }} />
              <label testID="size-label" text={`Size: ${size}`} />
              <select testID="fruit-select" options={fruits} selectedIndex={fruitIndex}
                onSelectionChanged={(e) => setFruitIndex(e.index)} />
              <label testID="fruit-label" text={`Fruit: ${fruits[fruitIndex]}`} />
              <slider testID="volume-slider" min={0} max={100} step={1} value={volume}
                onValueChanged={(e) => setVolume(e.value)} />
              <progressbar testID="volume-progress" fraction={volume / 100} />
              <label testID="volume-label" text={`Volume: ${volume}`} />
              <box orientation="horizontal" spacing={6}>
                <spinner testID="busy-spinner" spinning={true} />
                <image testID="smile-icon" iconName="face-smile-symbolic" />
                <webview testID="web-stub" />
              </box>
            </box>
            <grid tabLabel="Grid" testID="layout-grid">
              <label gridRow={0} gridColumn={0} text="r0c0" />
              <label gridRow={0} gridColumn={1} text="r0c1" />
              <label gridRow={1} gridColumn={0} gridColumnSpan={2} text="r1 span2" />
            </grid>
            <box tabLabel="Styled" orientation="vertical" spacing={6} testID="styled-tab">
              <label testID="styled-label" text="Styled label"
                style={{ background: "#2266cc", color: "#ffffff", padding: 8, margin: 4,
                         font: { fontSize: 16, fontWeight: "bold" },
                         border: { borderWidth: 2, borderColor: "#003399", borderRadius: 6 } }} />
              <button testID="styled-button" label="Styled button"
                style={{ background: "#cc2222", color: "#ffffff", padding: 6 }} />
            </box>
            <listview tabLabel="List" testID="big-list"
              items={rows}
              selectedIndex={selectedRow}
              onRowActivated={(e) => setActivatedRow(e.index)} />
            <box tabLabel="SourceList" orientation="vertical" spacing={6}>
              <sourcelist testID="gallery-sourcelist"
                items={sourceItems}
                selectedIndex={sourceSelected}
                onSelectionChanged={(e) => setSourceSelected(e.index)} />
              <label testID="sourcelist-selected-label" text={`SourceList selected: ${sourceSelected}`} />
            </box>

            <box tabLabel="Controls" orientation="vertical" spacing={10} testID="controls-tab" style={{ padding: 12 }}>
              <label text="ToggleButton" cssClasses={["heading"]} />
              <box orientation="horizontal" spacing={8}>
                <togglebutton testID="bold-toggle" label="Bold" active={bold} onToggled={(e) => setBold(e.checked)} />
                <label testID="bold-preview" text="The quick brown fox"
                  style={{ font: { fontWeight: bold ? "bold" : "normal" }, valign: "center" }} />
              </box>

              <separator orientation="horizontal" />

              <label text="SegmentedControl" cssClasses={["heading"]} />
              <box orientation="horizontal" spacing={8}>
                <segmentedcontrol testID="size-segmented" options={sizeOptions} selectedIndex={sizeSegmentIndex}
                  onSelectionChanged={(e) => setSizeSegmentIndex(e.index)} />
                <label testID="size-preview" text="Aa"
                  style={{ font: { fontSize: sizePreviewPt[sizeSegmentIndex] }, valign: "center" }} />
              </box>

              <separator orientation="horizontal" />

              <label text="NumberInput" cssClasses={["heading"]} />
              <box orientation="horizontal" spacing={8}>
                <numberinput testID="seats-number" value={seats} min={1} max={10} step={1} digits={0}
                  onValueChanged={(e) => setSeats(e.value)} />
                <label testID="seats-label" text={`Seats: ${seats}`} style={{ valign: "center" }} />
              </box>

              <separator orientation="horizontal" />

              <label text="LinkButton" cssClasses={["heading"]} />
              <box orientation="horizontal" spacing={8}>
                {/* openExternal defaults to false, so activating this never
                    spawns a real browser — onActivate still fires with the
                    uri as its payload, which is what drives the readout. */}
                <linkbutton testID="docs-link" label="NativeDesktop Docs" uri="https://nativedesktop.dev"
                  onActivate={(e) => setLastLinkActivated(e.text)} />
                <label testID="link-activated-label" text={`Activated: ${lastLinkActivated || "(not yet)"}`}
                  cssClasses={["dimmed", "caption"]} style={{ valign: "center" }} />
              </box>
            </box>

            <box tabLabel="Pickers" orientation="vertical" spacing={10} testID="pickers-tab" style={{ padding: 12 }}>
              <label text="ColorPicker" cssClasses={["heading"]} />
              <box orientation="horizontal" spacing={8}>
                <colorpicker testID="color-picker" value={color} supportsAlpha
                  onColorChanged={(e) => setColor(e.text)} />
                <label testID="color-readout" text={`Color: ${color}`} style={{ valign: "center" }} />
              </box>

              <separator orientation="horizontal" />

              <label text="DatePicker" cssClasses={["heading"]} />
              <box orientation="vertical" spacing={8}>
                <datepicker testID="date-picker" value={pickedDate} displayStyle="calendar"
                  onDateChanged={(e) => setPickedDate(e.text)} />
                <label testID="date-readout" text={`Date: ${pickedDate || "(none picked)"}`}
                  cssClasses={["dimmed", "caption"]} />
              </box>

              <separator orientation="horizontal" />

              <label text="FontPicker" cssClasses={["heading"]} />
              <box orientation="horizontal" spacing={8}>
                <fontpicker testID="font-picker" value={fontDesc} onFontChanged={(e) => setFontDesc(e.text)} />
                <label testID="font-readout" text={`Font: ${fontDesc}`} style={{ valign: "center" }} />
              </box>

              <separator orientation="horizontal" />

              <label text="LevelIndicator (driven by Slider)" cssClasses={["heading"]} />
              <box orientation="vertical" spacing={6}>
                <slider testID="level-slider" min={0} max={1} step={0.01} value={levelValue}
                  onValueChanged={(e) => setLevelValue(e.value)} />
                <levelindicator testID="level-indicator" min={0} max={1} value={levelValue}
                  warningValue={0.6} criticalValue={0.85} />
                <label testID="level-readout" text={`Level: ${levelValue.toFixed(2)}`} cssClasses={["dimmed", "caption"]} />
              </box>
            </box>

            <box tabLabel="Popovers & Menus" orientation="vertical" spacing={10} testID="popovers-tab" style={{ padding: 12 }}>
              <label text="Popover" cssClasses={["heading"]} />
              {/* anchorRef names the widget to present from, so the popover's
                  place in the tree decides nothing. Without it a popover
                  falls back to anchoring on its tree parent, which is why
                  it used to have to sit in a box beside its own button. */}
              <box orientation="horizontal" spacing={8}>
                <button ref={popoverTrigger} testID="popover-trigger" label="Open Popover" onClick={() => setPopoverOpen(true)} />
              </box>
              <box orientation="horizontal" spacing={8}>
                <popover testID="demo-popover" anchorRef={popoverTrigger} open={popoverOpen} position="bottom" onClosed={() => setPopoverOpen(false)}>
                  <box orientation="vertical" spacing={8} style={{ padding: 12 }}>
                    <label text="Popover content" />
                    <button testID="popover-close" label="Close" onClick={() => setPopoverOpen(false)} />
                  </box>
                </popover>
              </box>

              <separator orientation="horizontal" />

              <label text="MenuButton" cssClasses={["heading"]} />
              <menubutton testID="menu-button-demo" label="Actions" iconName="open-menu">
                <menuitem testID="menu-action-duplicate" label="Duplicate" onSelect={() => setLastMenuAction("Duplicate")} />
                <menuitem testID="menu-action-rename" label="Rename" onSelect={() => setLastMenuAction("Rename")} />
                <menuitem role="separator" testID="menu-action-sep" />
                <menuitem testID="menu-action-delete" label="Delete" iconName="edit-delete" onSelect={() => setLastMenuAction("Delete")} />
              </menubutton>

              <label text="SplitButton" cssClasses={["heading"]} />
              <splitbutton testID="split-button-demo" label="Save" iconName="document-save"
                onClick={() => setLastMenuAction("Save (primary action)")}>
                <menuitem testID="split-action-save-as" label="Save As…" onSelect={() => setLastMenuAction("Save As…")} />
                <menuitem testID="split-action-save-copy" label="Save a Copy" onSelect={() => setLastMenuAction("Save a Copy")} />
              </splitbutton>

              <label testID="menu-action-readout" text={`Last menu action: ${lastMenuAction || "(none)"}`}
                cssClasses={["dimmed", "caption"]} />

              <separator orientation="horizontal" />

              <label text="Expander" cssClasses={["heading"]} />
              <expander testID="more-options-expander" label="More options" expanded={expanderOpen}
                onToggled={(e) => setExpanderOpen(e.checked)}>
                <box orientation="vertical" spacing={6} style={{ padding: 8 }}>
                  <label text="Hidden until expanded." cssClasses={["dimmed"]} />
                  <checkbox testID="expander-check" label="An option inside the expander" checked={expanderChecked}
                    onToggled={(e) => setExpanderChecked(e.checked)} />
                </box>
              </expander>
            </box>

            <box tabLabel="Status & Banner" orientation="vertical" spacing={10} testID="status-tab" style={{ padding: 12 }}>
              <label text="Banner" cssClasses={["heading"]} />
              <box orientation="horizontal" spacing={8}>
                <label text="Show banner" style={{ valign: "center" }} />
                <switch testID="banner-reveal-switch" checked={bannerRevealed} onToggled={(e) => setBannerRevealed(e.checked)} />
              </box>
              <banner testID="update-banner" title="A new version is available" buttonLabel="Update Now"
                revealed={bannerRevealed} onButtonClicked={() => setBannerRevealed(false)} />

              <separator orientation="horizontal" />

              <label text="StatusPage" cssClasses={["heading"]} />
              <statuspage testID="empty-status-page" iconName="folder" title="No files yet"
                description="Add your first file to get started.">
                <button testID="status-page-action" label="Add File" onClick={() => setStatusActionCount((c) => c + 1)} />
              </statuspage>
              <label testID="status-action-readout" text={`Add File clicked: ${statusActionCount} time(s)`}
                cssClasses={["dimmed", "caption"]} />
            </box>

            <box tabLabel="Toasts" orientation="vertical" spacing={10} testID="toasts-tab" style={{ padding: 12 }}>
              <label text="ToastOverlay wraps this whole window — these buttons call showToast() on it."
                cssClasses={["dimmed", "caption"]} />
              <button testID="toast-simple-button" label="Show Toast" onClick={async () => {
                const result = await showToast(toastRef.current!, { title: "Saved successfully" });
                setLastToastResult(`buttonClicked: ${result.buttonClicked}`);
              }} />
              <button testID="toast-action-button" label="Show Toast with Action" onClick={async () => {
                const result = await showToast(toastRef.current!, {
                  title: "File deleted",
                  buttonLabel: "Undo",
                  timeoutSeconds: 6,
                });
                setLastToastResult(`buttonClicked: ${result.buttonClicked}`);
              }} />
              <label testID="toast-result-readout" text={`Last toast result: ${lastToastResult || "(none yet)"}`}
                cssClasses={["dimmed", "caption"]} />
            </box>

            <box tabLabel="Table" orientation="vertical" spacing={8} testID="table-tab" style={{ padding: 12 }}>
              <label text="Click a column header to sort — sorting happens in JS (sortChanged), GTK never reorders rows on its own."
                cssClasses={["dimmed", "caption"]} />
              <table
                testID="employee-table"
                columns={tableColumns}
                rows={employees.map((e): TableRow => ({
                  id: e.id,
                  cells: [e.name, e.role, e.department, String(e.years), e.status],
                }))}
                selectedIndex={selectedEmployeeIndex}
                showRowSeparators
                onSelectionChanged={(e) => setSelectedEmployeeIndex(e.index)}
                onRowActivated={(e) => setActivatedEmployeeIndex(e.index)}
                onSortChanged={handleSortChanged}
                style={{ vexpand: true }}
              />
              <label testID="table-selection-readout"
                text={`Selected: ${selectedEmployeeIndex >= 0 ? (employees[selectedEmployeeIndex]?.name ?? "(none)") : "(none)"} · Activated row: ${activatedEmployeeIndex}`}
                cssClasses={["dimmed", "caption"]} />
              <label testID="table-sort-readout" text={`Last sort: ${lastSort || "(unsorted)"}`}
                cssClasses={["dimmed", "caption"]} />
            </box>

            <box tabLabel="Tree" orientation="vertical" spacing={8} testID="tree-tab" style={{ padding: 12 }}>
              <treeview
                testID="category-tree"
                nodes={treeNodes}
                indentationPerLevel={16}
                onSelectionChanged={(e) => setSelectedTreeNodeId((e.data as { nodeId: string | null }).nodeId)}
                onRowActivated={(e) => setActivatedTreeNodeId((e.data as { nodeId: string | null }).nodeId)}
                onNodeExpanded={(e) => {
                  const { nodeId } = e.data as { nodeId: string };
                  setTreeExpanded((prev) => new Set(prev).add(nodeId));
                }}
                onNodeCollapsed={(e) => {
                  const { nodeId } = e.data as { nodeId: string };
                  setTreeExpanded((prev) => {
                    const next = new Set(prev);
                    next.delete(nodeId);
                    return next;
                  });
                }}
                style={{ vexpand: true }}
              />
              <label testID="tree-selection-readout"
                text={`Selected: ${treeNodeTitle(selectedTreeNodeId)} · Activated: ${treeNodeTitle(activatedTreeNodeId)}`}
                cssClasses={["dimmed", "caption"]} />
            </box>

            <box tabLabel="SourceTree" orientation="vertical" spacing={8} testID="sourcetree-tab" style={{ padding: 12 }}>
              {/* `toolbar` is a structural class on BOTH backends now: GTK's
                  Adwaita toolbar styling, AppKit's .headerView material strip
                  with a bottom hairline. */}
              <box orientation="horizontal" spacing={6} cssClasses={["toolbar"]} testID="st-toolbar">
                <button testID="st-toolbar-refresh" iconName="view-refresh-symbolic" cssClasses={["flat"]} />
                <button testID="st-toolbar-add" iconName="list-add-symbolic" cssClasses={["flat"]} />
              </box>
              <sourcetree
                testID="gallery-sourcetree-tree"
                nodes={stNodes}
                actions={sourceTreeActions}
                selectedId={stSelectedId}
                onSelectionChanged={(e) => setStSelectedId((e.data as { nodeId: string | null }).nodeId ?? "")}
                onRowActivated={(e) => setStLastActivated((e.data as { nodeId: string }).nodeId)}
                onNodeExpanded={(e) => {
                  const { nodeId } = e.data as { nodeId: string };
                  setStLastExpandEvent(`expanded:${nodeId}`);
                  setStExpanded((prev) => new Set(prev).add(nodeId));
                }}
                onNodeCollapsed={(e) => {
                  const { nodeId } = e.data as { nodeId: string };
                  setStLastExpandEvent(`collapsed:${nodeId}`);
                  setStExpanded((prev) => {
                    const next = new Set(prev);
                    next.delete(nodeId);
                    return next;
                  });
                }}
                onActionClicked={(e) => {
                  const { nodeId, actionId } = e.data as { nodeId: string; actionId: string };
                  setStLastAction(`${actionId}@${nodeId}`);
                }}
                style={{ vexpand: true }}
              />
              <checkbox testID="st-settled-toggle" label="Show settled" checked={stExpanded.has("sec-settled")}
                onToggled={(e) => setStExpanded((prev) => {
                  const next = new Set(prev);
                  if (e.checked) next.add("sec-settled");
                  else next.delete("sec-settled");
                  return next;
                })} />
              <label testID="st-selected-readout" text={`Selected: ${stSelectedId || "(none)"}`}
                cssClasses={["dimmed", "caption"]} />
              <label testID="st-activated-readout" text={`Activated: ${stLastActivated || "(none)"}`}
                cssClasses={["dimmed", "caption"]} />
              <label testID="st-action-readout" text={`Action: ${stLastAction || "(none)"}`}
                cssClasses={["dimmed", "caption"]} />
              <label testID="st-expand-readout" text={`Expand event: ${stLastExpandEvent || "(none)"}`}
                cssClasses={["dimmed", "caption"]} />
            </box>

            <box tabLabel="Dialogs" orientation="vertical" spacing={10} testID="dialogs-tab" style={{ padding: 12 }}>
              <box orientation="horizontal" spacing={8}>
                <button testID="show-alert-button" label="Show Alert" onClick={handleShowAlert} />
                <label testID="alert-result-readout" text={`Result: ${alertResultText}`}
                  cssClasses={["dimmed", "caption"]} style={{ valign: "center" }} />
              </box>
              <box orientation="horizontal" spacing={8}>
                <button testID="open-file-button" label="Open File…" onClick={handleOpenFile} />
                <label testID="open-file-result-readout" text={`Result: ${openFileResultText}`}
                  cssClasses={["dimmed", "caption"]} style={{ valign: "center" }} />
              </box>
              <box orientation="horizontal" spacing={8}>
                <button testID="save-file-button" label="Save File…" onClick={handleSaveFile} />
                <label testID="save-file-result-readout" text={`Result: ${saveFileResultText}`}
                  cssClasses={["dimmed", "caption"]} style={{ valign: "center" }} />
              </box>
              <button testID="show-about-button" label="About This App" onClick={handleShowAbout} />
            </box>

            <box tabLabel="Video" orientation="vertical" spacing={8} testID="video-tab" style={{ padding: 12 }}>
              <label text="Bundled sample clip below; Choose Video File… swaps in any local video via the same openFile() dialog used in the Dialogs tab."
                cssClasses={["dimmed", "caption"]} />
              <video testID="gallery-video" src={videoSrc} controls loop />
              <button testID="choose-video-button" label="Choose Video File…" onClick={handleChooseVideo} />
              <label testID="video-src-readout" text={`Source: ${videoSrc}`} cssClasses={["dimmed", "caption"]} />
            </box>

            <box tabLabel="macOS" orientation="vertical" spacing={10} testID="macos-tab" style={{ padding: 12 }}>
              {/* Platform.os, not Platform.backend — TrayItem/ShareButton are
                  OS-level concepts (NSStatusItem, NSSharingServicePicker)
                  that only exist as native APIs on macOS, independent of
                  which rendering backend (GTK-via-Quartz or AppKit) is
                  driving this particular process. */}
              {Platform.os === "macos" ? (
                <>
                  <label text="TrayItem" cssClasses={["heading"]} />
                  <trayitem testID="tray-item-demo" iconName="face-smile-symbolic" tooltip="NativeDesktop Gallery" />
                  <label testID="tray-item-note"
                    text="A TrayItem node is mounted (visible in the automation tree). AppKit renders a real NSStatusItem; GTK has no native tray concept, so this backend mounts an invisible placeholder box."
                    cssClasses={["dimmed", "caption"]} />

                  <separator orientation="horizontal" />

                  <label text="ShareButton" cssClasses={["heading"]} />
                  <sharebutton testID="share-button-demo" label="Share" items={["https://nativedesktop.dev"]} />
                  <label testID="share-button-note"
                    text="Same story: AppKit shows the native NSSharingServicePicker button; GTK mounts an invisible placeholder (there is no GTK share-sheet equivalent)."
                    cssClasses={["dimmed", "caption"]} />
                </>
              ) : (
                <label testID="macos-only-label" text="TrayItem and ShareButton are macOS-only widgets. Not available on this platform."
                  cssClasses={["dimmed"]} />
              )}
            </box>
          </tabview>
          <label testID="activated-label" text={`Activated: ${activatedRow}`} />
          <scrollview testID="log-scroll" minContentHeight={120}>
            <box orientation="vertical" spacing={2}>
              {Array.from({ length: 40 }, (_, i) => <label key={i} text={`Row ${i}`} />)}
            </box>
          </scrollview>
        </box>
      </toastoverlay>
    </window>
  );
}

await render(<App />);
