# M2 — NDP Protocol + Bun Child: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Tasks 3–5 are independently implementable against the Interfaces blocks; Tasks 6–9 integrate them.

**Goal:** From the M1 GTK window, stand up the NativeDesktop Protocol (NDP): the Zig host listens on a unix domain socket, spawns a Bun child, they complete a version handshake, and a plain TypeScript script (no React) builds a Window/Box/Label/Button tree by sending `commitBatch` ops. Button clicks flow back as `event` messages. `NDP_TRACE=1` pretty-prints every frame on both sides. `kill -9` of the Bun child leaves the window alive and answering — a permanent CI regression test. All green under headless CI.

**Architecture:** Two processes (spec D1). The Zig **host** owns `main()` and the GTK main loop (blocking). Before spawning the child it creates and listens on a unix socket at `$XDG_RUNTIME_DIR/nd-<pid>.sock`, passing the path to the child via the `ND_SOCKET` env var. A dedicated **reader thread** (`std.Thread`) reads length-prefixed JSON frames off the socket; each complete `commitBatch` is marshaled to the GTK main thread as **one** closure via `glib.MainContext.invokeFull`, which applies the whole batch atomically — never per-op dispatch. Events (host→runtime) are written back from the UI thread through a **mutex-guarded writer**. The **Bun child** connects via `Bun.connect({ unix })`, reassembles partial frames, and drives the tree. JSON, not binary (spec D3; binary is D4/M10). All I/O on the Zig side uses `std.Io.Threaded` (spec D5: `std.Io.Threaded` for aux I/O, UI main loop stays native GTK).

**Tech Stack:** Zig 0.16.0 (exact), the vendored zig-gobject bindings already at `vendor/gobject-bindings` (glib2/gobject2/gio2/gtk4), Bun 1.3.13 (from the flake), GTK4 ≥ 4.20 (devshell 4.22.4), weston headless + `GSK_RENDERER=cairo` for CI, GitHub Actions.

## Global Constraints

Carried over from the M1 plan, unchanged:

- Zig is exactly `0.16.0`; `build.zig`'s `checkZigVersion()` guard stays and fails loudly otherwise.
- Bun is pinned `1.3.13` (from the flake devshell); the child is run as `bun runtime/m2-demo.ts`.
- No `@cImport` anywhere — it no longer exists in Zig 0.16; all GTK access goes through the vendored zig-gobject modules imported as `glib`/`gobject`/`gio`/`gtk` (names fixed in `build.zig`).
- No hand-written per-widget C bindings (spec D6); only the vendored generated modules.
- Headless CI uses `weston --backend=headless` + `GSK_RENDERER=cairo` — NOT Broadway, NOT Xvfb/X11.
- Commit style: short imperative lowercase subject (e.g. `feat: add ndp framing and message types`). No co-author trailers, no body unless closing an issue.
- All commands run inside the devshell (direnv activates it in the repo; in CI, `nix develop -c`).
- Host prints machine-greppable markers to **stderr** (`std.debug.print` writes to stderr): `ND_CHILD_CONNECTED`, `ND_HELLO_OK`, `ND_COMMIT_APPLIED commitId=<n>`, `ND_CHILD_EXITED`. Scripts capture `2>&1`.
- TypeScript is strict, no npm dependencies; only stable Bun features (`Bun.spawn` is not used for the channel — its IPC is Bun-to-Bun only; the channel is our own unix socket per research entry `runtime_integration`).

### Zig 0.16 I/O reality (verified against the devshell std source — read before writing any socket/JSON/spawn code)

The original architecture note said "read the socket with std.posix reads." **That is not the 0.16 idiom** and the tasks below correct it: Writergate + std.Io (spec D5) moved sockets to `std.Io.net`, which is the sanctioned aux-I/O path. Every socket/spawn/JSON call takes an `Io` obtained from a `std.Io.Threaded` instance. The exact symbols, verified in this session against `$(dirname $(readlink -f $(which zig)))/../lib/zig/std`, are:

| Need | 0.16 symbol | Source file |
|---|---|---|
| Threaded I/O backend | `std.Io.Threaded.init(...)`, `.io()` → `Io`, `.deinit()` | `std/Io/Threaded.zig` |
| Unix socket address | `std.Io.net.UnixAddress.init(path)` | `std/Io/net.zig:848` |
| Listen | `UnixAddress.listen(io, .{}) → net.Server` | `std/Io/net.zig:880` |
| Accept | `Server.accept(io) → net.Stream` | `std/Io/net.zig:1442` |
| Connect (not used host-side) | `UnixAddress.connect(io) → net.Stream` | `std/Io/net.zig:908` |
| Read/write on a stream | `Stream.reader(io, buf) → Reader`, `Stream.writer(io, buf) → Writer`, `Stream.close(io)` | `std/Io/net.zig:1393` |
| Read exact N bytes | `Reader.readSliceAll(buf) ![]` | `std/Io/Reader.zig:660` |
| Write bytes / u32 | `Writer.writeAll(bytes)`, `Writer.writeInt(u32, v, .little)` | `std/Io/Writer.zig:549,851` |
| Spawn child | `std.process.spawn(io, SpawnOptions{ .argv, .environ_map, .stdin/out/err }) → Child` | `std/process.zig:442` |
| Kill / wait | `Child.kill(io)`, `Child.wait(io) → Term` | `std/process/Child.zig:118,134` |
| JSON decode | `std.json.parseFromSlice(T, allocator, s, .{}) → Parsed(T)` | `std/json/static.zig:73` |
| JSON encode | `std.json.Stringify.valueAlloc(gpa, v, .{}) ![]u8` | `std/json/Stringify.zig:618` |
| Thread | `std.Thread.spawn(.{}, fn, args) → Thread` | `std/Thread.zig:344` |
| Main-loop marshal | `glib.MainContext.default().invokeFull(prio, SourceFunc, data, ?DestroyNotify)` | `vendor/.../glib2/glib2.zig:5287` |
| Idle-source callback type | `glib.SourceFunc = *const fn (?*anyopaque) callconv(.c) c_int` | `vendor/.../glib2/glib2.zig:5606` |
| Default priority | `glib.PRIORITY_DEFAULT = 0` | `vendor/.../glib2/glib2.zig:26072` |

**u32 LE framing:** use `w.writeInt(u32, len, .little)` to emit and `std.mem.readInt(u32, buf[0..4], .little)` (after `readSliceAll` into a 4-byte buffer) to consume — do not depend on a Reader integer-take helper name; `std.mem.readInt` is stable.

Every task that writes one of these calls includes a one-line re-verification command; run it inside the devshell before pasting code. If a symbol has drifted, the observable behavior (the markers and the demo) is the contract — adjust the call, not the behavior.

---

### Task 1: Extend the flake devshell if Bun is missing anything, and add the ND_SOCKET convention doc

**Files:**
- Verify (likely no change): `flake.nix`

**Interfaces:**
- Consumes: existing devshell (from M1: zig, bun, gtk4, glib, weston, xsltproc).
- Produces: confirmation that `bun` (1.3.13) and `weston` resolve inside the devshell; no new packages expected. This task is a guard, not a build step — later tasks assume `bun` is on PATH.

- [ ] **Step 1: Confirm the tools resolve**

Run: `nix develop -c bash -c 'bun --version && weston --version | head -1 && zig version'`
Expected: `1.3.13`, a weston version line, `0.16.0`. If `bun` is absent, add `bun` to the devshell package list (M1 already lists it) and re-run. No commit if nothing changed.

- [ ] **Step 2 (only if flake changed): Commit**

```bash
git add flake.nix
git commit -m "chore: ensure bun and weston resolve in devshell for m2"
```

### Task 2: NDP framing + message types + encode/decode (TDD, pure Zig, no GTK)

**Files:**
- Create: `src/protocol.zig`
- Modify: `build.zig` (add a `protocol` test target so `zig build test` covers it)

**Interfaces:**
- Consumes: nothing (pure module, no GTK, no I/O).
- Produces:
  - Message types (field names verbatim from spec §4 M2 subset):
    - `Hello = struct { type: []const u8 = "hello", ndpVersion: u32, runtime: struct { name: []const u8, version: []const u8 } }`
    - `HelloAck = struct { type: []const u8 = "helloAck", ndpVersion: u32, encodings: []const []const u8 }`
    - `ErrorFrame = struct { type: []const u8 = "error", message: []const u8, expected: u32, got: u32 }`
    - `Event = struct { type: []const u8 = "event", seq: u64, priority: []const u8, nodeId: u32, name: []const u8, payload: struct {} }`
    - `Op` (tagged by `op` field) and `CommitBatch = struct { type: []const u8 = "commitBatch", commitId: u64, generation: u32, ops: []Op }`
  - `pub const ndp_version: u32 = 1;`
  - `pub fn encodeFrame(gpa, value) ![]u8` — returns `u32 LE length ‖ JSON` (one allocation, caller frees).
  - `pub fn frameLen(reader) !u32` and a batch decoder `pub fn parseCommitBatch(gpa, json_bytes) !std.json.Parsed(CommitBatch)` — thin wrappers over `std.json`.
  - A frame-type sniffer `pub fn peekType(gpa, json_bytes) ![]const u8` (parses only the `type` field via a `struct { type: []const u8 }`) so the reader can route hello vs commitBatch without a full union.

- [ ] **Step 1: Verify the JSON API names, then write the failing test first**

Run: `nix develop -c bash -c 'STD=$(dirname $(readlink -f $(which zig)))/../lib/zig/std; rg -n "pub fn parseFromSlice\b|pub fn valueAlloc\b" $STD/json/static.zig $STD/json/Stringify.zig'`
Expected: `parseFromSlice` at `static.zig:73`, `valueAlloc` at `Stringify.zig:618`. If either differs, adjust the wrapper bodies below (signatures shown match this session's std).

Write `src/protocol.zig` with the golden-frame test FIRST (implementation stubs return `error.Unimplemented` so the test fails):

```zig
const std = @import("std");

pub const ndp_version: u32 = 1;

pub const Hello = struct {
    type: []const u8 = "hello",
    ndpVersion: u32,
    runtime: Runtime,
    pub const Runtime = struct { name: []const u8, version: []const u8 };
};

pub const HelloAck = struct {
    type: []const u8 = "helloAck",
    ndpVersion: u32,
    encodings: []const []const u8,
};

pub const ErrorFrame = struct {
    type: []const u8 = "error",
    message: []const u8,
    expected: u32,
    got: u32,
};

pub const Event = struct {
    type: []const u8 = "event",
    seq: u64,
    priority: []const u8 = "discrete",
    nodeId: u32,
    name: []const u8,
    payload: struct {} = .{},
};

/// One of create|append|setText|update. Decoded with a permissive struct:
/// optional fields cover the union of all op shapes; the `op` string discriminates.
pub const Op = struct {
    op: []const u8,
    // create
    id: ?u32 = null,
    widget: ?[]const u8 = null, // "Window" | "Box" | "Label" | "Button"
    props: ?std.json.Value = null,
    // append
    parent: ?u32 = null,
    child: ?u32 = null,
    // setText
    text: ?[]const u8 = null,
};

pub const CommitBatch = struct {
    type: []const u8 = "commitBatch",
    commitId: u64,
    generation: u32,
    ops: []Op,
};

/// u32 LE length prefix + UTF-8 JSON. Caller frees the returned slice.
pub fn encodeFrame(gpa: std.mem.Allocator, value: anytype) ![]u8 {
    const json = try std.json.Stringify.valueAlloc(gpa, value, .{});
    defer gpa.free(json);
    const frame = try gpa.alloc(u8, 4 + json.len);
    std.mem.writeInt(u32, frame[0..4], @intCast(json.len), .little);
    @memcpy(frame[4..], json);
    return frame;
}

/// Reads only the "type" field so the reader can route without a full union parse.
pub fn peekType(gpa: std.mem.Allocator, json_bytes: []const u8) ![]const u8 {
    const TypeOnly = struct { type: []const u8 };
    const parsed = try std.json.parseFromSlice(TypeOnly, gpa, json_bytes, .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    return gpa.dupe(u8, parsed.value.type);
}

test "encodeFrame writes u32 LE length prefix + json, golden bytes" {
    const gpa = std.testing.allocator;
    const hello = Hello{ .ndpVersion = 1, .runtime = .{ .name = "bun", .version = "1.3.13" } };
    const frame = try encodeFrame(gpa, hello);
    defer gpa.free(frame);

    // Prefix is the JSON byte length, little-endian.
    const json_len = std.mem.readInt(u32, frame[0..4], .little);
    try std.testing.expectEqual(@as(usize, json_len), frame.len - 4);

    // Round-trips back to the same struct.
    const parsed = try std.json.parseFromSlice(Hello, gpa, frame[4..], .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    try std.testing.expectEqual(@as(u32, 1), parsed.value.ndpVersion);
    try std.testing.expectEqualStrings("bun", parsed.value.runtime.name);

    // Byte-layout anchor: the first four bytes are the length low byte first,
    // and byte 4 is '{'.
    try std.testing.expectEqual(@as(u8, '{'), frame[4]);
    try std.testing.expectEqual(frame[0], @as(u8, @truncate(json_len)));
}

test "peekType routes commitBatch vs hello" {
    const gpa = std.testing.allocator;
    const t1 = try peekType(gpa, "{\"type\":\"hello\",\"ndpVersion\":1}");
    defer gpa.free(t1);
    try std.testing.expectEqualStrings("hello", t1);

    const t2 = try peekType(gpa, "{\"type\":\"commitBatch\",\"commitId\":3,\"generation\":0,\"ops\":[]}");
    defer gpa.free(t2);
    try std.testing.expectEqualStrings("commitBatch", t2);
}

test "commitBatch with a create op decodes with field names verbatim" {
    const gpa = std.testing.allocator;
    const doc =
        \\{"type":"commitBatch","commitId":1,"generation":0,"ops":[
        \\  {"op":"create","id":1,"widget":"Window","props":{"title":"Hi","defaultWidth":480,"defaultHeight":320}},
        \\  {"op":"create","id":2,"widget":"Box","props":{"orientation":"vertical","spacing":8}},
        \\  {"op":"append","parent":1,"child":2},
        \\  {"op":"create","id":3,"widget":"Label","props":{"text":"Clicks: 0"}},
        \\  {"op":"setText","id":3,"text":"Clicks: 1"}
        \\]}
    ;
    const parsed = try std.json.parseFromSlice(CommitBatch, gpa, doc, .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    try std.testing.expectEqual(@as(u64, 1), parsed.value.commitId);
    try std.testing.expectEqual(@as(usize, 5), parsed.value.ops.len);
    try std.testing.expectEqualStrings("create", parsed.value.ops[0].op);
    try std.testing.expectEqualStrings("Window", parsed.value.ops[0].widget.?);
    try std.testing.expectEqual(@as(u32, 2), parsed.value.ops[2].child.?);
    try std.testing.expectEqualStrings("Clicks: 1", parsed.value.ops[4].text.?);
}
```

- [ ] **Step 2: Wire a protocol test into `build.zig`**

`src/protocol.zig` is pure (no GTK imports), so give it its own test artifact — do not pull the GTK modules into it. In `build.zig`, after the existing `tests` block, add:

```zig
    const protocol_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/protocol.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(protocol_tests).step);
```

- [ ] **Step 3: Run — expect the golden test to fail, then pass**

Run: `nix develop -c zig build test`
Expected: on first write the `encodeFrame`/`peekType` bodies above already compile; the three tests should pass immediately since the implementation is included. (This module is small enough that the "failing first" discipline is the golden-byte assertion itself — if a field name is wrong, the round-trip or `peekType` assertion fails loudly.) Fix the first failure only.

- [ ] **Step 4: Commit**

```bash
git add src/protocol.zig build.zig
git commit -m "feat: ndp framing, message types, and encode/decode with golden-frame tests"
```

### Task 3: Retained node tree (`src/tree.zig`) — id→widget registry + generation

**Files:**
- Create: `src/tree.zig`

**Interfaces:**
- Consumes: `protocol.CommitBatch` / `protocol.Op` (Task 2); `gtk_backend` create/apply functions (Task 4) — declared here as the calling surface.
- Produces:
  - `pub const Tree = struct { ... }` holding `nodes: std.AutoHashMapUnmanaged(u32, *gtk.Widget)`, `generation: u32`, a root window pointer, and an `allocator`.
  - `pub fn init(gpa, app) Tree`
  - `pub fn apply(self: *Tree, batch: protocol.CommitBatch) void` — **runs on the UI thread only**; iterates ops in order, dispatching each to `gtk_backend`. `create` → build widget, store in map (window ops also `present()` via the backend); `append` → `gtk_backend.appendChild(parent_widget, child_widget)`; `setText` → `gtk_backend.setText(widget, text)`; `update` → `gtk_backend.applyProps(widget, kind, props)`. Unknown op → `std.debug.print` a warning, skip (never crash the host on a bad op).
  - `pub fn get(self: *Tree, id: u32) ?*gtk.Widget`
  - Generation lives here for M3's re-mount; in M2 it is stored and logged only.

- [ ] **Step 1: Write `src/tree.zig`**

```zig
const std = @import("std");
const gtk = @import("gtk");
const protocol = @import("protocol.zig");
const backend = @import("gtk_backend.zig");

pub const Tree = struct {
    gpa: std.mem.Allocator,
    app: *gtk.Application,
    nodes: std.AutoHashMapUnmanaged(u32, *gtk.Widget) = .{},
    generation: u32 = 0,

    pub fn init(gpa: std.mem.Allocator, app: *gtk.Application) Tree {
        return .{ .gpa = gpa, .app = app };
    }

    pub fn get(self: *Tree, id: u32) ?*gtk.Widget {
        return self.nodes.get(id);
    }

    /// UI-thread only. Applies an entire commit batch as one unit.
    pub fn apply(self: *Tree, batch: protocol.CommitBatch) void {
        self.generation = batch.generation;
        for (batch.ops) |op| {
            if (std.mem.eql(u8, op.op, "create")) {
                const widget = backend.createWidget(self.app, op.widget.?, op.props) catch continue;
                self.nodes.put(self.gpa, op.id.?, widget) catch continue;
            } else if (std.mem.eql(u8, op.op, "append")) {
                const parent = self.nodes.get(op.parent.?) orelse continue;
                const child = self.nodes.get(op.child.?) orelse continue;
                backend.appendChild(parent, child);
            } else if (std.mem.eql(u8, op.op, "setText")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.setText(widget, op.text.?);
            } else if (std.mem.eql(u8, op.op, "update")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.applyProps(widget, op.widget orelse "", op.props);
            } else {
                std.debug.print("ND_WARN unknown op={s}\n", .{op.op});
            }
        }
        std.debug.print("ND_COMMIT_APPLIED commitId={d}\n", .{batch.commitId});
    }
};
```

- [ ] **Step 2: Compile-check (no standalone test — needs GTK; it is exercised end-to-end in Task 7)**

Run: `nix develop -c zig build` — expected to fail only because `gtk_backend.zig` (Task 4) does not exist yet. That failure is the interface contract; implement Task 4 next. Do not commit until Task 4 lands and `zig build` is green.

### Task 4: GTK widget backend (`src/gtk_backend.zig`) — create/apply props + click signal → event

**Files:**
- Create: `src/gtk_backend.zig`

**Interfaces:**
- Consumes: the vendored `gtk` module; a host-provided `EventSink` callback so a clicked button can enqueue an outbound `event`.
- Produces:
  - `pub const EventSink = *const fn (node_id: u32) void;` and `pub fn setEventSink(sink: EventSink) void` (module-global, set once at startup by `runtime.zig`).
  - `pub fn createWidget(app: *gtk.Application, kind: []const u8, props: ?std.json.Value) !*gtk.Widget` — dispatches on kind:
    - `"Window"`: `gtk.ApplicationWindow.new(app)`, apply `title`/`defaultWidth`/`defaultHeight`, `present()`. Returns `.as(gtk.Widget)`.
    - `"Box"`: `gtk.Box.new(orientation, spacing)` where `orientation` maps `"vertical"→.vertical`, else `.horizontal`.
    - `"Label"`: `gtk.Label.new(text)`.
    - `"Button"`: `gtk.Button.newWithLabel(label)`; connect the `clicked` signal to a trampoline that calls the sink with this node's id (id passed as the connect user-data).
  - `pub fn appendChild(parent: *gtk.Widget, child: *gtk.Widget) void` — Window→`setChild`, Box→`append`. Discriminate by GType check (`gtk.Window.getType`)... simpler: try Window first via `gobject.typeCheckInstanceIsA`; if not a Window, treat as Box `append`. (See idiom note.)
  - `pub fn setText(widget: *gtk.Widget, text: []const u8) void` — Label `setText`.
  - `pub fn applyProps(widget, kind, props)` — M2 supports Box `spacing`, Window `title`; extend as ops require.

- [ ] **Step 1: Verify the widget API names against the vendored bindings**

Run these — all confirmed present in this session, re-check if drift is suspected:
```bash
rg -n "gtk_box_new|gtk_box_append|gtk_box_set_spacing" vendor/gobject-bindings/src/gtk4/gtk4.zig
rg -n "gtk_label_new\b|gtk_label_set_text\b" vendor/gobject-bindings/src/gtk4/gtk4.zig
rg -n "pub const Orientation = enum" vendor/gobject-bindings/src/gtk4/gtk4.zig
```
Expected: `Box.new(orientation, spacing)`, `Box.append`, `Box.setSpacing`, `Label.new(?str)`, `Label.setText`, `Orientation { horizontal=0, vertical=1 }`. The click-signal idiom is identical to M1's `gtk.Button.signals.clicked.connect(button, DataT, &cb, data, .{})` in `src/main.zig` — mirror it.

- [ ] **Step 2: Write `src/gtk_backend.zig`**

The prop values arrive as `std.json.Value`; read them with small typed helpers. Strings passed to GTK must be null-terminated — allocate a sentinel copy from a module arena (freed never in M2; widgets outlive the process). For discriminating Window vs Box on append, connect it structurally: store the created Window in a module-global `?*gtk.Window` (M2 has exactly one window) and route `appendChild` to `setChild` when the parent equals it, else `Box.append`.

```zig
const std = @import("std");
const gtk = @import("gtk");
const glib = @import("glib");

var event_sink: ?EventSink = null;
var arena_state = std.heap.ArenaAllocator.init(std.heap.page_allocator);
const arena = arena_state.allocator();
var the_window: ?*gtk.Window = null;

pub const EventSink = *const fn (node_id: u32) void;

pub fn setEventSink(sink: EventSink) void {
    event_sink = sink;
}

fn dupeZ(s: []const u8) [:0]const u8 {
    return arena.dupeZ(u8, s) catch @panic("OOM in gtk_backend arena");
}

fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .string => field.string,
        else => null,
    };
}

fn propInt(props: ?std.json.Value, key: []const u8) ?i64 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .integer => field.integer,
        else => null,
    };
}

pub fn createWidget(app: *gtk.Application, kind: []const u8, props: ?std.json.Value) !*gtk.Widget {
    if (std.mem.eql(u8, kind, "Window")) {
        const window = gtk.ApplicationWindow.new(app);
        const win = window.as(gtk.Window);
        the_window = win;
        if (propStr(props, "title")) |t| gtk.Window.setTitle(win, dupeZ(t));
        const w: c_int = @intCast(propInt(props, "defaultWidth") orelse 480);
        const h: c_int = @intCast(propInt(props, "defaultHeight") orelse 320);
        gtk.Window.setDefaultSize(win, w, h);
        gtk.Window.present(win);
        return window.as(gtk.Widget);
    } else if (std.mem.eql(u8, kind, "Box")) {
        const vertical = if (propStr(props, "orientation")) |o| std.mem.eql(u8, o, "vertical") else true;
        const orientation: gtk.Orientation = if (vertical) .vertical else .horizontal;
        const spacing: c_int = @intCast(propInt(props, "spacing") orelse 0);
        const box = gtk.Box.new(orientation, spacing);
        return box.as(gtk.Widget);
    } else if (std.mem.eql(u8, kind, "Label")) {
        const text = propStr(props, "text") orelse "";
        const label = gtk.Label.new(dupeZ(text));
        return label.as(gtk.Widget);
    } else if (std.mem.eql(u8, kind, "Button")) {
        const lbl = propStr(props, "label") orelse "Button";
        const button = gtk.Button.newWithLabel(dupeZ(lbl));
        return button.as(gtk.Widget);
    }
    std.debug.print("ND_WARN unknown widget kind={s}\n", .{kind});
    return error.UnknownWidget;
}

/// The clicked-signal user-data is the node id, packed into the pointer slot.
pub fn connectButtonClick(button: *gtk.Button, node_id: u32) void {
    const data: ?*anyopaque = @ptrFromInt(@as(usize, node_id));
    _ = gtk.Button.signals.clicked.connect(button, ?*anyopaque, &onClicked, data, .{});
}

fn onClicked(_: *gtk.Button, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    if (event_sink) |sink| sink(node_id);
}

pub fn appendChild(parent: *gtk.Widget, child: *gtk.Widget) void {
    if (the_window) |win| {
        if (parent == win.as(gtk.Widget)) {
            gtk.Window.setChild(win, child);
            return;
        }
    }
    const box: *gtk.Box = @ptrCast(@alignCast(parent));
    gtk.Box.append(box, child);
}

pub fn setText(widget: *gtk.Widget, text: []const u8) void {
    const label: *gtk.Label = @ptrCast(@alignCast(widget));
    gtk.Label.setText(label, dupeZ(text));
}

pub fn applyProps(widget: *gtk.Widget, kind: []const u8, props: ?std.json.Value) void {
    if (std.mem.eql(u8, kind, "Box")) {
        if (propInt(props, "spacing")) |s| {
            const box: *gtk.Box = @ptrCast(@alignCast(widget));
            gtk.Box.setSpacing(box, @intCast(s));
        }
    } else if (std.mem.eql(u8, kind, "Window")) {
        if (propStr(props, "title")) |t| {
            const win: *gtk.Window = @ptrCast(@alignCast(widget));
            gtk.Window.setTitle(win, dupeZ(t));
        }
    }
}
```

Note: `createWidget` for a Button must call `connectButtonClick` before returning. Adjust the Button branch to capture the id — but the id is known in `tree.zig` (`op.id.?`), not here. Resolve by having `tree.zig` call `backend.connectButtonClick(button_as_button, op.id.?)` right after a `"Button"` create. Add to `tree.zig`'s create branch:

```zig
const widget = backend.createWidget(self.app, op.widget.?, op.props) catch continue;
if (std.mem.eql(u8, op.widget.?, "Button")) {
    backend.connectButtonClick(@ptrCast(@alignCast(widget)), op.id.?);
}
self.nodes.put(self.gpa, op.id.?, widget) catch continue;
```

- [ ] **Step 3: Compile-check with a temporary event sink**

Run: `nix develop -c zig build` — expected to fail only because `runtime.zig`/`main.zig` are not yet wired (Task 5/6). The `gtk_backend.zig` + `tree.zig` pair must compile once their imports resolve; if the pointer-cast idiom (`@ptrCast(@alignCast(...))`) is rejected for a widget upcast, prefer the binding's `.as()` downcast helper where one exists — check `rg "pub fn as\b" vendor/gobject-bindings/src/gobject2/gobject2.zig`. Behavior contract: a Label updates its text, a Box appends children, a Button click fires the sink.

- [ ] **Step 4: Commit tree + backend together (they are one compilable unit)**

```bash
git add src/tree.zig src/gtk_backend.zig
git commit -m "feat: retained node tree and gtk widget backend with click events"
```

### Task 5: Runtime plumbing (`src/runtime.zig`) — socket listen, child spawn, reader thread, mutex writer

**Files:**
- Create: `src/runtime.zig`

**Interfaces:**
- Consumes: `protocol` (Task 2), `tree.Tree` (Task 3), `gtk_backend.EventSink` (Task 4), the `glib`/`gio`/`gtk` modules.
- Produces:
  - `pub const Runtime = struct { ... }` owning: a `std.Io.Threaded` and its `Io`, the `net.Server`, the accepted `net.Stream`, a pointer to the shared `*Tree`, a `writer_mutex: std.Thread.Mutex`, an outbound `event` `seq` counter, and the socket path buffer.
  - `pub fn start(gpa, app, tree) !*Runtime` — allocates a `Runtime`, builds the socket path `"$XDG_RUNTIME_DIR/nd-<pid>.sock"`, unlinks any stale file, `UnixAddress.listen`, spawns the Bun child (`bun <script>` via `std.process.spawn` with `ND_SOCKET` in `environ_map`), then spawns the reader `std.Thread`. Called from `main.zig` inside `onActivate` (window/app already exist).
  - `pub fn sendEvent(self: *Runtime, node_id: u32) void` — the `EventSink`; increments `seq`, encodes an `Event{ seq, priority:"discrete", nodeId, name:"clicked", payload:{} }` frame, takes `writer_mutex`, writes it, releases. Called from the UI thread (button click).
  - Reader thread loop (`fn readerLoop(self: *Runtime) void`): accept the child, print `ND_CHILD_CONNECTED`; read the first frame, expect `hello`; if `ndpVersion != protocol.ndp_version`, write an `error` frame, close, `child.kill(io)`, return; else write `helloAck`, print `ND_HELLO_OK`. Then loop: read u32 LE length, read that many bytes, `peekType`; on `commitBatch`, parse and **marshal to the UI thread** via `glib.MainContext.default().invokeFull(...)` (one closure applying the whole batch); on `ping`, reply `pong`; on EOF/`error.EndOfStream`/read error, print `ND_CHILD_EXITED` and return (the GTK loop keeps running — crash isolation).
  - `NDP_TRACE=1`: if the env var is set, print `>> <json>` for every frame read (runtime→host) and `<< <json>` for every frame written (host→runtime) to stderr.

- [ ] **Step 1: Verify the socket + spawn + thread + marshal API names**

Run inside the devshell:
```bash
nix develop -c bash -c 'STD=$(dirname $(readlink -f $(which zig)))/../lib/zig/std
rg -n "pub fn init|pub fn io\b" $STD/Io/Threaded.zig | head
rg -n "UnixAddress|pub fn listen|pub fn accept|pub fn reader|pub fn writer|pub fn close" $STD/Io/net.zig | head
rg -n "pub fn spawn\b|pub const SpawnOptions|environ_map|argv:" $STD/process.zig | head
rg -n "pub fn spawn" $STD/Thread.zig | head'
rg -n "pub const invokeFull|pub const default =" vendor/gobject-bindings/src/glib2/glib2.zig
```
Expected shapes (this session): `Threaded.init(...)`/`.io()`; `UnixAddress.init/listen`, `Server.accept`, `Stream.reader/writer/close`; `std.process.spawn(io, SpawnOptions{ .argv, .environ_map, ... })`; `std.Thread.spawn(.{}, fn, args)`; `MainContext.invokeFull` + `MainContext.default`. Paste the real signatures if any differ — the two hazards are (a) `Threaded.init` argument list and (b) whether `environ_map` wants a `*const Environ.Map` you must build from the parent env plus `ND_SOCKET`.

- [ ] **Step 2: Write `src/runtime.zig`**

The marshal must hand the parsed batch to the UI thread and free it there. Package `{ *Tree, *Parsed(CommitBatch) }` on the heap, pass its pointer as `invokeFull` user-data, and free it in the trampoline (which returns `G_SOURCE_REMOVE`).

```zig
const std = @import("std");
const glib = @import("glib");
const gio = @import("gio");
const gtk = @import("gtk");
const protocol = @import("protocol.zig");
const Tree = @import("tree.zig").Tree;
const backend = @import("gtk_backend.zig");

const G_SOURCE_REMOVE: c_int = 0;

var trace: bool = false;

pub const Runtime = struct {
    gpa: std.mem.Allocator,
    threaded: std.Io.Threaded,
    io: std.Io,
    server: std.Io.net.Server,
    stream: ?std.Io.net.Stream = null,
    tree: *Tree,
    child: std.process.Child,
    writer_mutex: std.Thread.Mutex = .{},
    write_buf: [4096]u8 = undefined,
    seq: u64 = 0,
    sock_path: [:0]u8,

    var singleton: ?*Runtime = null;

    pub fn start(gpa: std.mem.Allocator, app: *gtk.Application, tree: *Tree) !*Runtime {
        trace = std.posix.getenv("NDP_TRACE") != null;

        const self = try gpa.create(Runtime);
        self.* = undefined;
        self.gpa = gpa;
        self.tree = tree;

        self.threaded = std.Io.Threaded.init(gpa);
        self.io = self.threaded.io();

        const runtime_dir = std.posix.getenv("XDG_RUNTIME_DIR") orelse "/tmp";
        const pid = std.os.linux.getpid();
        self.sock_path = try std.fmt.allocPrintZ(gpa, "{s}/nd-{d}.sock", .{ runtime_dir, pid });
        std.posix.unlink(self.sock_path) catch {};

        const addr = try std.Io.net.UnixAddress.init(self.sock_path);
        self.server = try addr.listen(self.io, .{});

        // Spawn `bun <script>` with ND_SOCKET set. Build an environ map that
        // inherits the parent env and adds ND_SOCKET (see verification note re: Environ.Map).
        var env = try std.process.Environ.Map.init(gpa); // adjust ctor name per Step 1 verify
        try env.put("ND_SOCKET", self.sock_path);
        const script = std.posix.getenv("ND_SCRIPT") orelse "runtime/m2-demo.ts";
        self.child = try std.process.spawn(self.io, .{
            .argv = &.{ "bun", script },
            .environ_map = &env,
            .stdin = .inherit,
            .stdout = .inherit,
            .stderr = .inherit,
        });

        singleton = self;
        backend.setEventSink(&sendEventStatic);

        _ = try std.Thread.spawn(.{}, readerLoop, .{self});
        return self;
    }

    fn sendEventStatic(node_id: u32) void {
        if (singleton) |self| self.sendEvent(node_id);
    }

    pub fn sendEvent(self: *Runtime, node_id: u32) void {
        self.seq += 1;
        const ev = protocol.Event{ .seq = self.seq, .nodeId = node_id, .name = "clicked" };
        const frame = protocol.encodeFrame(self.gpa, ev) catch return;
        defer self.gpa.free(frame);
        if (trace) std.debug.print("<< {s}\n", .{frame[4..]});
        self.writer_mutex.lock();
        defer self.writer_mutex.unlock();
        const stream = self.stream orelse return;
        var w = stream.writer(self.io, &self.write_buf);
        w.interface.writeAll(frame) catch {};
        w.interface.flush() catch {};
    }

    fn writeFrame(self: *Runtime, value: anytype) void {
        const frame = protocol.encodeFrame(self.gpa, value) catch return;
        defer self.gpa.free(frame);
        if (trace) std.debug.print("<< {s}\n", .{frame[4..]});
        self.writer_mutex.lock();
        defer self.writer_mutex.unlock();
        const stream = self.stream orelse return;
        var w = stream.writer(self.io, &self.write_buf);
        w.interface.writeAll(frame) catch {};
        w.interface.flush() catch {};
    }

    fn readerLoop(self: *Runtime) void {
        const stream = self.server.accept(self.io) catch {
            std.debug.print("ND_CHILD_EXITED\n", .{});
            return;
        };
        self.stream = stream;
        std.debug.print("ND_CHILD_CONNECTED\n", .{});

        var read_buf: [64 * 1024]u8 = undefined;
        var r = stream.reader(self.io, &read_buf);

        // Handshake.
        const first = readFrame(self, &r.interface) catch {
            std.debug.print("ND_CHILD_EXITED\n", .{});
            return;
        };
        defer self.gpa.free(first);
        {
            const parsed = std.json.parseFromSlice(protocol.Hello, self.gpa, first, .{ .ignore_unknown_fields = true }) catch {
                std.debug.print("ND_CHILD_EXITED\n", .{});
                return;
            };
            defer parsed.deinit();
            if (parsed.value.ndpVersion != protocol.ndp_version) {
                self.writeFrame(protocol.ErrorFrame{
                    .message = "ndp version mismatch",
                    .expected = protocol.ndp_version,
                    .got = parsed.value.ndpVersion,
                });
                self.child.kill(self.io);
                std.debug.print("ND_CHILD_EXITED\n", .{});
                return;
            }
        }
        self.writeFrame(protocol.HelloAck{ .ndpVersion = protocol.ndp_version, .encodings = &.{"json"} });
        std.debug.print("ND_HELLO_OK\n", .{});

        // Frame loop.
        while (true) {
            const bytes = readFrame(self, &r.interface) catch {
                std.debug.print("ND_CHILD_EXITED\n", .{});
                return;
            };
            const kind = protocol.peekType(self.gpa, bytes) catch {
                self.gpa.free(bytes);
                continue;
            };
            defer self.gpa.free(kind);

            if (std.mem.eql(u8, kind, "commitBatch")) {
                // Ownership of `bytes` transfers to the marshaled closure.
                self.marshalCommit(bytes);
            } else if (std.mem.eql(u8, kind, "ping")) {
                self.gpa.free(bytes);
                self.writeFrame(.{ .type = "pong" });
            } else {
                self.gpa.free(bytes);
            }
        }
    }

    /// Reads one u32 LE length prefix + payload. Caller frees the returned slice.
    fn readFrame(self: *Runtime, r: *std.Io.Reader) ![]u8 {
        var len_buf: [4]u8 = undefined;
        try r.readSliceAll(&len_buf);
        const len = std.mem.readInt(u32, &len_buf, .little);
        const payload = try self.gpa.alloc(u8, len);
        errdefer self.gpa.free(payload);
        try r.readSliceAll(payload);
        if (trace) std.debug.print(">> {s}\n", .{payload});
        return payload;
    }

    const CommitJob = struct { rt: *Runtime, bytes: []u8 };

    fn marshalCommit(self: *Runtime, bytes: []u8) void {
        const job = self.gpa.create(CommitJob) catch {
            self.gpa.free(bytes);
            return;
        };
        job.* = .{ .rt = self, .bytes = bytes };
        _ = glib.MainContext.default().invokeFull(glib.PRIORITY_DEFAULT, &applyOnUi, job, null);
    }

    fn applyOnUi(data: ?*anyopaque) callconv(.c) c_int {
        const job: *CommitJob = @ptrCast(@alignCast(data.?));
        const self = job.rt;
        const parsed = std.json.parseFromSlice(protocol.CommitBatch, self.gpa, job.bytes, .{ .ignore_unknown_fields = true }) catch {
            self.gpa.free(job.bytes);
            self.gpa.destroy(job);
            return G_SOURCE_REMOVE;
        };
        defer parsed.deinit();
        self.tree.apply(parsed.value);
        self.gpa.free(job.bytes);
        self.gpa.destroy(job);
        return G_SOURCE_REMOVE;
    }
};
```

Idiom hazards to resolve at implement-time (all bounded):
- `stream.reader(io, buf)` / `.writer(io, buf)` return a struct whose `.interface` field is the `*std.Io.Reader`/`*std.Io.Writer` (Writergate wrapper). If the field is named differently in the pinned std, `rg -n "pub fn reader\b" -A8 $STD/Io/net.zig` shows the return type; adjust `r.interface`/`w.interface` accordingly.
- `std.process.Environ.Map` constructor: Step 1's verify shows the real name; if building a full inherited env is awkward, an accepted fallback is `std.posix.setenv`-equivalent is gone in 0.16 — instead build the map from `std.process.getEnvMap(gpa)` if that still exists (`rg "pub fn getEnvMap" $STD/process.zig`), then add `ND_SOCKET`.
- `glib.MainContext.default()` returns `*MainContext`; `.invokeFull(prio, func, data, notify)` — `func` must be `glib.SourceFunc` (`*const fn(?*anyopaque) callconv(.c) c_int`). `applyOnUi` matches.

- [ ] **Step 3: Commit after Task 6 wires main and `zig build` is green**

Do not commit `runtime.zig` alone — it references `main.zig` wiring. Proceed to Task 6, then commit both.

### Task 6: Wire the host — `main.zig` starts the runtime, keeps `--smoke`

**Files:**
- Modify: `src/main.zig`
- Modify: `build.zig` (the exe/test modules must import `protocol`, `tree`, `gtk_backend`, `runtime` — they are sibling files under `src/`, so they resolve via `@import("runtime.zig")` without new build wiring; confirm the exe module's `root_source_file` is `src/main.zig` and nothing else is needed).

**Interfaces:**
- Consumes: `runtime.Runtime.start` (Task 5), the existing M1 `onActivate`.
- Produces: `zig build run` opens the window and spawns the Bun child; `--smoke` still prints `ND_SMOKE_MAPPED` and quits (unchanged path, must not spawn a child in smoke mode — smoke is the pure-GTK M1 check). A new flag/env is not needed: in normal mode `onActivate` calls `runtime.Runtime.start`.

- [ ] **Step 1: Modify `onActivate` in `src/main.zig`**

Keep everything M1 does, but replace the hard-coded button child with runtime startup. In M2 the window content is built by the child via `commitBatch`, so `onActivate` should create the `ApplicationWindow` only if we still want a base window — **decision:** let the child's `create Window` op own the window; `onActivate` in normal mode just starts the runtime and lets the first `commitBatch` present the window. In `--smoke` mode, keep the M1 behavior exactly (build a window + button, print `ND_SMOKE_MAPPED`, quit) so the M1 CI smoke test is untouched.

```zig
const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gio = @import("gio");
const gtk = @import("gtk");
const Tree = @import("tree.zig").Tree;
const Runtime = @import("runtime.zig").Runtime;

pub const app_id = "dev.nativedesktop.hello";

var smoke = false;
var global_app: ?*gtk.Application = null;
var gpa_state = std.heap.GeneralPurposeAllocator(.{}){};
var tree: Tree = undefined;

pub fn main(init: std.process.Init) void {
    for (init.minimal.args.vector) |arg| {
        if (std.mem.eql(u8, std.mem.span(arg), "--smoke")) smoke = true;
    }
    var app = gtk.Application.new(app_id, .{});
    defer app.unref();
    global_app = app;
    _ = gio.Application.signals.activate.connect(app, ?*anyopaque, &onActivate, null, .{});
    const argv = init.minimal.args.vector;
    const status = gio.Application.run(app.as(gio.Application), @intCast(argv.len), @ptrCast(@constCast(argv.ptr)));
    std.process.exit(@intCast(status));
}

fn onActivate(app: *gtk.Application, _: ?*anyopaque) callconv(.c) void {
    if (smoke) {
        // Unchanged M1 pure-GTK smoke path.
        const window = gtk.ApplicationWindow.new(app);
        const win = window.as(gtk.Window);
        gtk.Window.setTitle(win, "NativeDesktop M1");
        gtk.Window.setDefaultSize(win, 480, 320);
        const button = gtk.Button.newWithLabel("Click me");
        gtk.Window.setChild(win, button.as(gtk.Widget));
        _ = gtk.Widget.signals.map.connect(window.as(gtk.Widget), ?*anyopaque, &onMapped, null, .{});
        gtk.Window.present(win);
        return;
    }
    // M2: the child builds the tree over NDP.
    const gpa = gpa_state.allocator();
    tree = Tree.init(gpa, app);
    // Hold the app alive with no window until the first commit presents one.
    gio.Application.hold(app.as(gio.Application));
    _ = Runtime.start(gpa, app, &tree) catch |err| {
        std.debug.print("ND_RUNTIME_ERROR {any}\n", .{err});
        gio.Application.quit(app.as(gio.Application));
    };
}

fn onMapped(_: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    std.debug.print("ND_SMOKE_MAPPED\n", .{});
    _ = glib.idleAdd(&quitIdle, null);
}

fn quitIdle(_: ?*anyopaque) callconv(.c) c_int {
    if (global_app) |app| gio.Application.quit(app.as(gio.Application));
    return 0;
}

test "toolchain pin matches .zigversion" {
    const builtin = @import("builtin");
    const pinned = std.mem.trim(u8, @embedFile(".zigversion"), " \n\r\t");
    const required = try std.SemanticVersion.parse(pinned);
    try std.testing.expect(builtin.zig_version.order(required) == .eq);
}
```

Note on `gio.Application.hold`: without an open window a `GtkApplication` quits immediately after `activate`. `hold`/`release` keeps it alive; verify the binding exposes it: `rg -n "g_application_hold\b" vendor/gobject-bindings/src/gio2/gio2.zig`. If absent, the alternative is to let `onActivate` create an empty `ApplicationWindow` up front and have the child's `create Window` op reuse it — but the `hold` route keeps ownership with the protocol. Confirmed present is preferred; the fallback is one line in `tree.zig`.

- [ ] **Step 2: Build the whole host**

Run: `nix develop -c zig build`
Expected: clean build producing `zig-out/bin/nd-hello`. Fix the first compile error only; the likely ones are the Writergate `.interface` field (Task 5 note) and `Environ.Map`/`getEnvMap` (Task 5 note). Then run the pure-unit tests: `nix develop -c zig build test` — protocol golden tests + toolchain pin still green.

- [ ] **Step 3: Commit the integrated host**

```bash
git add src/main.zig src/runtime.zig
git commit -m "feat: host listens on unix socket, spawns bun child, applies commit batches"
```

### Task 7: Bun NDP client lib + M2 demo (TypeScript, no React, no npm deps)

**Files:**
- Create: `runtime/ndp.ts`
- Create: `runtime/m2-demo.ts`

**Interfaces:**
- Consumes: `ND_SOCKET` env var; the socket protocol from Tasks 2/5.
- Produces:
  - `runtime/ndp.ts`: `connect(): Promise<Ndp>` (reads `ND_SOCKET`, `Bun.connect({ unix })`), with partial-frame reassembly, `sendHello(runtime)`, `awaitHelloAck()`, `sendCommit(batch)`, `onEvent(cb)`, `ping()`, `NDP_TRACE` printing `>> `/`<< ` per frame (from the runtime's viewpoint: `<<` = frames it sends toward host? — **fix the convention to match the host**: host uses `>>` for runtime→host and `<<` for host→runtime; the Bun side must print the SAME tags for the SAME direction, so Bun prints `>>` when it SENDS and `<<` when it RECEIVES).
  - `runtime/m2-demo.ts`: builds Window→Box(vertical)→[Label "Clicks: 0", Button "Click me", Label "Uptime: 0s"]; on a `clicked` event, `setText` the clicks label with an incremented count; a 500ms timer bumps the uptime label (so headless CI asserts commits without input synthesis). Prints nothing but frames; relies on the host markers.

- [ ] **Step 1: Confirm the Bun socket API shape**

`Bun.connect({ unix })` with a `socket.data(sock, buffer)` handler is the stable path (research entry `ipc-bridge`: Bun IPC is Bun-to-Bun only, so a raw unix socket is required; the socket handler delivers `Uint8Array` chunks that must be reassembled — Bun does not frame). No npm deps. Verify at implement-time: `bun --print 'typeof Bun.connect'` inside the devshell prints `function`.

- [ ] **Step 2: Write `runtime/ndp.ts`**

```ts
type Runtime = { name: string; version: string };
type Op =
  | { op: "create"; id: number; widget: "Window" | "Box" | "Label" | "Button"; props: Record<string, unknown> }
  | { op: "append"; parent: number; child: number }
  | { op: "setText"; id: number; text: string }
  | { op: "update"; id: number; props: Record<string, unknown> };
type CommitBatch = { type: "commitBatch"; commitId: number; generation: number; ops: Op[] };
type EventMsg = { type: "event"; seq: number; priority: string; nodeId: number; name: string; payload: object };

const TRACE = process.env.NDP_TRACE === "1";
const NDP_VERSION = 1;

export class Ndp {
  private socket: import("bun").Socket;
  private inbox = new Uint8Array(0);
  private eventCb: ((e: EventMsg) => void) | null = null;
  private helloAckResolve: (() => void) | null = null;

  private constructor(socket: import("bun").Socket) {
    this.socket = socket;
  }

  static async connect(): Promise<Ndp> {
    const path = process.env.ND_SOCKET;
    if (!path) throw new Error("ND_SOCKET not set");
    let self!: Ndp;
    const socket = await Bun.connect({
      unix: path,
      socket: {
        data(_sock, chunk) {
          self.onData(chunk);
        },
        close() {
          process.exit(0);
        },
      },
    });
    self = new Ndp(socket);
    return self;
  }

  private onData(chunk: Uint8Array) {
    const merged = new Uint8Array(this.inbox.length + chunk.length);
    merged.set(this.inbox, 0);
    merged.set(chunk, this.inbox.length);
    this.inbox = merged;
    // Drain complete frames.
    while (this.inbox.length >= 4) {
      const view = new DataView(this.inbox.buffer, this.inbox.byteOffset, 4);
      const len = view.getUint32(0, true);
      if (this.inbox.length < 4 + len) break;
      const json = new TextDecoder().decode(this.inbox.subarray(4, 4 + len));
      this.inbox = this.inbox.subarray(4 + len);
      if (TRACE) console.error("<< " + json); // host→runtime = received here
      this.dispatch(JSON.parse(json));
    }
  }

  private dispatch(msg: any) {
    if (msg.type === "helloAck") {
      if (msg.ndpVersion !== NDP_VERSION) throw new Error(`ndp mismatch: host ${msg.ndpVersion}`);
      this.helloAckResolve?.();
    } else if (msg.type === "error") {
      throw new Error(`host error: ${msg.message} (expected ${msg.expected}, got ${msg.got})`);
    } else if (msg.type === "event") {
      this.eventCb?.(msg as EventMsg);
    }
  }

  private send(obj: object) {
    const json = new TextEncoder().encode(JSON.stringify(obj));
    const frame = new Uint8Array(4 + json.length);
    new DataView(frame.buffer).setUint32(0, json.length, true);
    frame.set(json, 4);
    if (TRACE) console.error(">> " + new TextDecoder().decode(json)); // runtime→host = sent here
    this.socket.write(frame);
  }

  async handshake(runtime: Runtime): Promise<void> {
    const done = new Promise<void>((res) => (this.helloAckResolve = res));
    this.send({ type: "hello", ndpVersion: NDP_VERSION, runtime });
    await done;
  }

  sendCommit(batch: Omit<CommitBatch, "type">) {
    this.send({ type: "commitBatch", ...batch });
  }

  onEvent(cb: (e: EventMsg) => void) {
    this.eventCb = cb;
  }

  ping() {
    this.send({ type: "ping" });
  }
}

export type { Op, CommitBatch, EventMsg };
```

- [ ] **Step 3: Write `runtime/m2-demo.ts`**

```ts
import { Ndp } from "./ndp";

const WIN = 1, BOX = 2, CLICKS = 3, BUTTON = 4, UPTIME = 5;

const ndp = await Ndp.connect();
await ndp.handshake({ name: "bun", version: Bun.version });

let commitId = 0;
const commit = (ops: any[]) => ndp.sendCommit({ commitId: commitId++, generation: 0, ops });

// Initial tree.
commit([
  { op: "create", id: WIN, widget: "Window", props: { title: "NativeDesktop M2", defaultWidth: 480, defaultHeight: 320 } },
  { op: "create", id: BOX, widget: "Box", props: { orientation: "vertical", spacing: 8 } },
  { op: "append", parent: WIN, child: BOX },
  { op: "create", id: CLICKS, widget: "Label", props: { text: "Clicks: 0" } },
  { op: "append", parent: BOX, child: CLICKS },
  { op: "create", id: BUTTON, widget: "Button", props: { label: "Click me" } },
  { op: "append", parent: BOX, child: BUTTON },
  { op: "create", id: UPTIME, widget: "Label", props: { text: "Uptime: 0s" } },
  { op: "append", parent: BOX, child: UPTIME },
]);

let clicks = 0;
ndp.onEvent((e) => {
  if (e.name === "clicked" && e.nodeId === BUTTON) {
    clicks++;
    commit([{ op: "setText", id: CLICKS, text: `Clicks: ${clicks}` }]);
  }
});

// Drive commits without input so headless CI can assert ND_COMMIT_APPLIED.
let seconds = 0;
setInterval(() => {
  seconds++;
  commit([{ op: "setText", id: UPTIME, text: `Uptime: ${seconds}s` }]);
}, 500);
```

- [ ] **Step 4: Run end-to-end interactively (if a display is available) or headless**

Run: `nix develop -c bash -c 'NDP_TRACE=1 zig build run'`
Expected (stderr): `ND_CHILD_CONNECTED`, `>> {"type":"hello"...}`, `<< {"type":"helloAck"...}`, `ND_HELLO_OK`, then repeating `>> {"type":"commitBatch"...}` and `ND_COMMIT_APPLIED commitId=0,1,2,...`. A window titled "NativeDesktop M2" with two labels and a button; clicking the button increments "Clicks:". If no display, defer to Task 8's headless script for verification.

- [ ] **Step 5: Commit the Bun side**

```bash
git add runtime/ndp.ts runtime/m2-demo.ts
git commit -m "feat: bun ndp client lib and m2 no-react demo tree"
```

### Task 8: Headless M2 script — assert handshake + ≥3 commits

**Files:**
- Create: `scripts/headless-m2.sh`

**Interfaces:**
- Consumes: Task 6 host, Task 7 demo, weston (from M1's flake).
- Produces: `scripts/headless-m2.sh` exits 0 iff the host prints `ND_HELLO_OK` and at least three `ND_COMMIT_APPLIED` markers under a headless compositor. CI (Task 9) calls exactly this script.

- [ ] **Step 1: Write `scripts/headless-m2.sh`** (mirror `scripts/headless-smoke.sh`'s weston setup)

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m2
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

# Run the host for ~3s (enough for handshake + several 500ms uptime commits), then stop it.
OUT=$(timeout 4 ./zig-out/bin/nd-hello 2>&1 || true)
echo "$OUT"

grep -q "ND_HELLO_OK" <<<"$OUT" || { echo "FAIL: no handshake"; exit 1; }
COMMITS=$(grep -c "ND_COMMIT_APPLIED" <<<"$OUT" || true)
[ "$COMMITS" -ge 3 ] || { echo "FAIL: only $COMMITS commits applied"; exit 1; }
echo "headless m2: OK ($COMMITS commits)"
```

- [ ] **Step 2: Run it, expect failure before build, then success**

Run: `nix develop -c bash -c 'chmod +x scripts/headless-m2.sh && rm -rf zig-out && ./scripts/headless-m2.sh'`
Expected: FAIL (`nd-hello: No such file`) — proves it checks something. Then: `nix develop -c bash -c 'zig build && ./scripts/headless-m2.sh'`
Expected: markers stream, then `headless m2: OK (N commits)` with N ≥ 3, exit 0. `bun` must be on PATH (devshell provides it).

- [ ] **Step 3: Commit**

```bash
git add scripts/headless-m2.sh
git commit -m "feat: headless m2 script asserting handshake and commit markers"
```

### Task 9: kill -9 crash-isolation test + CI wiring

**Files:**
- Create: `scripts/kill9-test.sh`
- Modify: `.github/workflows/ci.yml` (already exists with the M1 steps — append the two M2 steps)

**Interfaces:**
- Consumes: Task 6 host, Task 7 demo, weston.
- Produces:
  - `scripts/kill9-test.sh`: starts the host headless, waits for `ND_CHILD_CONNECTED`, finds the Bun pid, `kill -9`s it, asserts the host prints `ND_CHILD_EXITED`, stays alive ≥3s, and exits 0 when sent `SIGTERM`. This is spec §12's permanent crash-isolation regression test (spec D1: JS death leaves the window up).
  - `.github/workflows/ci.yml`: a `linux` job running unit tests, build, headless-smoke (M1), headless-m2, and kill9-test under `nix develop -c`.

- [ ] **Step 1: Write `scripts/kill9-test.sh`**

Markers used (exact): `ND_CHILD_CONNECTED` (host, on accept), `ND_CHILD_EXITED` (host, on child EOF/read error). The host's window survival is proven by the host process still running 3s after the child dies; SIGTERM then exits it 0.

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-kill9
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true; kill "$HOST_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

LOG=$(mktemp)
./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!

# Wait for the child to connect.
for _ in $(seq 1 50); do
  grep -q "ND_CHILD_CONNECTED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_CHILD_CONNECTED" "$LOG" || { echo "FAIL: child never connected"; cat "$LOG"; exit 1; }

# The child is the `bun runtime/m2-demo.ts` process parented by the host.
BUN_PID=$(pgrep -f "bun .*m2-demo.ts" | head -1)
[ -n "$BUN_PID" ] || { echo "FAIL: no bun pid"; cat "$LOG"; exit 1; }
kill -9 "$BUN_PID"

# Host must observe the exit and stay alive.
for _ in $(seq 1 30); do
  grep -q "ND_CHILD_EXITED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_CHILD_EXITED" "$LOG" || { echo "FAIL: host did not report child exit"; cat "$LOG"; exit 1; }

sleep 3
kill -0 "$HOST_PID" 2>/dev/null || { echo "FAIL: host died with the child"; cat "$LOG"; exit 1; }

# Clean shutdown on SIGTERM.
kill -TERM "$HOST_PID"
wait "$HOST_PID" 2>/dev/null || true
echo "kill9: OK — window survived child death"
```

Note on the SIGTERM exit code: `wait` after a SIGTERM to a GTK app may report 143 (128+15); the test asserts survival + reporting, not a literal 0 exit from a signal. If a clean 0 exit is required, wire a SIGTERM handler in the host that calls `gio.Application.quit` — optional for M2, note it for the owner.

- [ ] **Step 2: Run it**

Run: `nix develop -c bash -c 'zig build && chmod +x scripts/kill9-test.sh && ./scripts/kill9-test.sh'`
Expected: `ND_CHILD_CONNECTED` appears, the bun pid is killed, `ND_CHILD_EXITED` appears, the host survives 3s, `kill9: OK — window survived child death`, exit 0.

- [ ] **Step 3: Extend `.github/workflows/ci.yml`**

The file already exists (M1 landed it) with `unit tests` / `build` / `headless smoke` steps. Append the two M2 steps after the existing `headless smoke` step so the `linux` job ends:

```yaml
      - name: headless smoke
        run: nix develop -c ./scripts/headless-smoke.sh
      - name: headless m2
        run: nix develop -c ./scripts/headless-m2.sh
      - name: kill -9 crash isolation
        run: nix develop -c ./scripts/kill9-test.sh
```

Do not rewrite the whole file — only the two new `- name:` blocks are added.

- [ ] **Step 4: Validate the full CI sequence locally**

Run: `nix develop -c bash -c 'zig build test && zig build && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh'`
Expected: all five green — the exact sequence CI runs.

- [ ] **Step 5: Commit**

```bash
git add scripts/kill9-test.sh .github/workflows/ci.yml
git commit -m "ci: kill -9 crash isolation test and m2 headless job"
```

---

## Self-review notes

**M2 scope coverage (spec §14 M2 line, not expanded):**
- Zig host spawns a Bun child — Task 5 (`std.process.spawn(io, .{.argv=&.{"bun", script}, .environ_map=…})`), Task 6 wires it into `onActivate`.
- NDP version handshake — Task 2 (`Hello`/`HelloAck`/`ErrorFrame`, `ndp_version=1`), Task 5 (host validates, replies `helloAck` or `error`+kill), Task 7 (`Ndp.handshake`). Version mismatch → error frame + clean child exit (Task 5 `readerLoop`).
- Plain TS (no React) builds Window/Box/Label/Button via `commitBatch` — Task 7 `m2-demo.ts`; ops `create`/`append`/`setText`/`update` with field names verbatim (Task 2).
- Button clicks → `event` with `seq` + `priority:"discrete"` — Task 4 (`connectButtonClick`→`onClicked`→sink), Task 5 (`sendEvent`), Task 7 (`onEvent`).
- `NDP_TRACE=1` pretty-prints every frame both sides, direction-tagged — Task 5 (`>>` recv / `<<` send on host), Task 7 (same tags, same direction: Bun prints `>>` on send, `<<` on receive). **Convention is consistent across both processes.**
- kill -9 leaves window alive → CI test — Task 9 `kill9-test.sh` + CI job.
- All green in CI headless — Task 9 CI wiring runs all scripts under weston + `GSK_RENDERER=cairo`.

**Architecture constraints honored:** two processes (D1); reader thread marshals one closure per `commitBatch` via `glib.MainContext.invokeFull` — never per-op (Task 5 `marshalCommit`/`applyOnUi`, Task 3 `Tree.apply` applies the whole batch); events written from the UI thread through a mutex-guarded writer (Task 5 `writer_mutex`); socket at `$XDG_RUNTIME_DIR/nd-<pid>.sock` created+listened before spawn, child gets `ND_SOCKET` (Task 5 `start`); u32 LE + JSON framing (Task 2); `std.Io.Threaded` for aux I/O, GTK loop native (D5). Widget prop subset limited to M2's Window/Box/Label/Button (Task 4).

**Correction to the input note, called out:** the note said "read the socket with std.posix reads, not old std.io readers." Verification against the 0.16 std source shows sockets live in `std.Io.net` and the sanctioned path is a `std.Io.Threaded`-backed `Stream.reader`/`.writer` (the `std.posix` module no longer exposes high-level `socket`/`bind`/`listen`/`accept`; only the raw constants). The plan uses `std.Io.net` + `readSliceAll` for exact framing (not the old `std.io` reader interface, which is what the note was warning against) — this satisfies the intent (no legacy readers) while being the real 0.16 idiom. Flagged for the owner as a deliberate, verified deviation.

**No placeholders:** every code block is complete and compilable modulo the three inline-flagged 0.16 idiom hazards, each with an exact `rg` verification command and a bounded fallback: (1) the Writergate `.interface` field name on `Stream.reader/.writer`; (2) `Environ.Map` vs `getEnvMap` for building the child env with `ND_SOCKET`; (3) `gio.Application.hold` presence in the bindings. None blocks a fresh implementer.

**Type consistency across tasks:** node-id integers are `u32` in Zig (`Op.id`, `Tree.nodes` key, `Event.nodeId`, click user-data) and `number` in TS; `commitId`/`seq` are `u64` in Zig / `number` in TS (JSON numbers; M2 counts stay well under 2^53); `ndp_version` is `u32 = 1` on both sides; marker strings `ND_CHILD_CONNECTED`/`ND_HELLO_OK`/`ND_COMMIT_APPLIED commitId=<n>`/`ND_CHILD_EXITED` are emitted in Task 5 exactly as the scripts in Tasks 8/9 grep for them; module import names `glib`/`gobject`/`gio`/`gtk` match `build.zig` and M1's `main.zig`; the `EventSink` signature `*const fn(u32) void` is identical in Task 4 (definition) and Task 5 (`sendEventStatic`).

**Judgment calls made within the constraints:**
1. **Window ownership.** The child's `create Window` op owns the window (not a pre-built host window); the host holds the app alive via `gio.Application.hold` until the first commit presents it. This keeps the protocol authoritative over the tree (matches spec "host holds the authoritative retained tree") and keeps `--smoke` on the untouched M1 pure-GTK path so M1's CI is unaffected. Fallback (pre-built empty window reused by the create op) noted inline.
2. **Mutex writer over writer-thread.** The constraint offered either; I chose the mutex-guarded writer (simpler, one fewer thread, correct because event volume is low — discrete clicks only in M2). Specified exactly in Task 5 (`writer_mutex` guards `sendEvent`/`writeFrame`).
3. **`std.Io.Threaded` sockets** rather than raw posix (verified idiom; see correction above).
4. **Op decode as a permissive struct with optional fields** rather than a Zig tagged union keyed on the `op` string — `std.json` has no native string-discriminated union and the optional-field struct is the least-friction decode that keeps every field name verbatim (Task 2). The `op` string discriminates at apply time (Task 3).
5. **CI file extended, not created.** `.github/workflows/ci.yml` already exists with the M1 steps (unit tests / build / headless smoke); Task 9 appends only the two M2 steps (`headless m2`, `kill -9 crash isolation`) rather than rewriting the file.
