//! The `<webview>` context-menu model: the item tree an app hands to
//! `setContextMenuItems`, and the hit-test matching that decides which of those
//! items a given right-click earns.
//!
//! Pure std, with no GTK and no WebKit, so the part that decides what the user
//! sees is covered by `zig build test`. That matters more here than elsewhere:
//! GTK4 synthesises no pointer input, so no automation can open the real menu
//! and read it back, and this file is the only place the selection is decidable.
const std = @import("std");

pub const Kind = enum { normal, checkbox, radio, separator };

/// Hit-test contexts an item can ask for, as bits so one mask answers both
/// sides of the match.
pub const CTX_PAGE: u8 = 1 << 0;
pub const CTX_LINK: u8 = 1 << 1;
pub const CTX_IMAGE: u8 = 1 << 2;
pub const CTX_SELECTION: u8 = 1 << 3;
pub const CTX_EDITABLE: u8 = 1 << 4;
pub const CTX_ALL: u8 = CTX_PAGE | CTX_LINK | CTX_IMAGE | CTX_SELECTION | CTX_EDITABLE;

pub const Item = struct {
    id: []const u8 = "",
    /// NUL-terminated because WebKit takes the label as a C string.
    label: [:0]const u8 = "",
    kind: Kind = .normal,
    checked: bool = false,
    enabled: bool = true,
    contexts: u8 = CTX_PAGE,
    /// `*`-wildcard globs tested against the hit's link or image URL. Empty
    /// means "any target".
    target_globs: [][]const u8 = &.{},
    children: []Item = &.{},
};

/// What the engine's hit test found under the pointer.
pub const Hit = struct {
    link: []const u8 = "",
    image: []const u8 = "",
    selection: []const u8 = "",
    editable: bool = false,
    /// WebKitGTK reports THAT there is a selection without its text, so this is
    /// the portable flag and `selection` is the AppKit-only extra.
    has_selection: bool = false,

    pub fn contexts(self: Hit) u8 {
        var bits: u8 = 0;
        if (self.link.len > 0) bits |= CTX_LINK;
        if (self.image.len > 0) bits |= CTX_IMAGE;
        if (self.has_selection or self.selection.len > 0) bits |= CTX_SELECTION;
        if (self.editable) bits |= CTX_EDITABLE;
        // Chrome's rule, and the one users read off a browser: "page" means the
        // click landed on nothing more specific.
        if (bits == 0) bits = CTX_PAGE;
        return bits;
    }
};

/// `*` matches any run of characters (including none); everything else is
/// literal. Deliberately NOT Chrome's match-pattern grammar: the framework has
/// no business knowing what an extension is, and a caller that has patterns
/// converts them to globs (see docs/webview.md).
pub fn globMatches(pattern: []const u8, text: []const u8) bool {
    var p: usize = 0;
    var t: usize = 0;
    var star: ?usize = null;
    var star_t: usize = 0;
    while (t < text.len) {
        if (p < pattern.len and (pattern[p] == text[t])) {
            p += 1;
            t += 1;
        } else if (p < pattern.len and pattern[p] == '*') {
            star = p;
            star_t = t;
            p += 1;
        } else if (star) |s| {
            p = s + 1;
            star_t += 1;
            t = star_t;
        } else return false;
    }
    while (p < pattern.len and pattern[p] == '*') p += 1;
    return p == pattern.len;
}

fn anyGlobMatches(globs: [][]const u8, url: []const u8) bool {
    if (url.len == 0) return false;
    for (globs) |g| {
        if (globMatches(g, url)) return true;
    }
    return false;
}

/// Does this item belong in the menu this hit earned?
pub fn matches(item: Item, hit: Hit) bool {
    if (item.kind == .separator) return true;
    const bits = hit.contexts() & item.contexts;
    if (bits == 0) return false;
    if (item.target_globs.len == 0) return true;
    // Which URL a target glob is tested against depends on which context
    // matched: a link item tests the href, an image item the source.
    if ((bits & CTX_LINK) != 0 and anyGlobMatches(item.target_globs, hit.link)) return true;
    if ((bits & CTX_IMAGE) != 0 and anyGlobMatches(item.target_globs, hit.image)) return true;
    return false;
}

/// True when this item, or anything under it, would be shown. A submenu whose
/// every child was filtered out is not a submenu, it is a dead label.
pub fn survives(item: Item, hit: Hit) bool {
    if (!matches(item, hit)) return false;
    if (item.children.len == 0) return true;
    for (item.children) |child| {
        if (child.kind == .separator) continue;
        if (survives(child, hit)) return true;
    }
    return false;
}

fn contextBit(name: []const u8) u8 {
    if (std.mem.eql(u8, name, "page")) return CTX_PAGE;
    if (std.mem.eql(u8, name, "link")) return CTX_LINK;
    if (std.mem.eql(u8, name, "image")) return CTX_IMAGE;
    if (std.mem.eql(u8, name, "selection")) return CTX_SELECTION;
    if (std.mem.eql(u8, name, "editable")) return CTX_EDITABLE;
    if (std.mem.eql(u8, name, "all")) return CTX_ALL;
    return 0;
}

fn kindOf(name: []const u8) Kind {
    if (std.mem.eql(u8, name, "checkbox")) return .checkbox;
    if (std.mem.eql(u8, name, "radio")) return .radio;
    if (std.mem.eql(u8, name, "separator")) return .separator;
    return .normal;
}

fn objStr(obj: std.json.ObjectMap, key: []const u8) ?[]const u8 {
    return switch (obj.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn objBool(obj: std.json.ObjectMap, key: []const u8) ?bool {
    return switch (obj.get(key) orelse return null) {
        .bool => |b| b,
        else => null,
    };
}

fn objArray(obj: std.json.ObjectMap, key: []const u8) ?std.json.Array {
    return switch (obj.get(key) orelse return null) {
        .array => |a| a,
        else => null,
    };
}

/// Parses the `setContextMenuItems` argument (`{ items: [...] }` or a bare
/// array) into an owned tree. Malformed entries are skipped rather than
/// failing the whole command: one bad item must not cost an app its menu.
pub fn parse(alloc: std.mem.Allocator, arg: std.json.Value) error{OutOfMemory}![]Item {
    const array = switch (arg) {
        .array => |a| a,
        .object => |o| objArray(o, "items") orelse return &.{},
        else => return &.{},
    };
    return parseArray(alloc, array);
}

fn parseArray(alloc: std.mem.Allocator, array: std.json.Array) error{OutOfMemory}![]Item {
    var out: std.ArrayList(Item) = .empty;
    errdefer {
        for (out.items) |item| freeItem(alloc, item);
        out.deinit(alloc);
    }
    for (array.items) |raw| {
        const obj = switch (raw) {
            .object => |o| o,
            else => continue,
        };
        const item = try parseItem(alloc, obj) orelse continue;
        try out.append(alloc, item);
    }
    return out.toOwnedSlice(alloc);
}

fn parseItem(alloc: std.mem.Allocator, obj: std.json.ObjectMap) error{OutOfMemory}!?Item {
    const kind = kindOf(objStr(obj, "type") orelse "normal");
    if (kind == .separator) return Item{ .kind = .separator, .contexts = CTX_ALL };

    const id = objStr(obj, "id") orelse return null;
    const label = objStr(obj, "label") orelse return null;
    if (id.len == 0 or label.len == 0) return null;

    var item = Item{
        .id = try alloc.dupe(u8, id),
        .label = try alloc.dupeZ(u8, label),
        .kind = kind,
        .checked = objBool(obj, "checked") orelse false,
        .enabled = objBool(obj, "enabled") orelse true,
    };
    errdefer freeItem(alloc, item);

    if (objArray(obj, "contexts")) |list| {
        var bits: u8 = 0;
        for (list.items) |entry| {
            switch (entry) {
                .string => |s| bits |= contextBit(s),
                else => {},
            }
        }
        if (bits != 0) item.contexts = bits;
    }

    if (objArray(obj, "targetUrlGlobs")) |list| {
        var globs: std.ArrayList([]const u8) = .empty;
        errdefer {
            for (globs.items) |g| alloc.free(g);
            globs.deinit(alloc);
        }
        for (list.items) |entry| {
            switch (entry) {
                .string => |s| try globs.append(alloc, try alloc.dupe(u8, s)),
                else => {},
            }
        }
        item.target_globs = try globs.toOwnedSlice(alloc);
    }

    if (objArray(obj, "children")) |list| item.children = try parseArray(alloc, list);
    return item;
}

pub fn freeItem(alloc: std.mem.Allocator, item: Item) void {
    if (item.id.len > 0) alloc.free(item.id);
    if (item.label.len > 0) alloc.free(item.label[0 .. item.label.len + 1]);
    for (item.target_globs) |g| alloc.free(g);
    if (item.target_globs.len > 0) alloc.free(item.target_globs);
    for (item.children) |child| freeItem(alloc, child);
    if (item.children.len > 0) alloc.free(item.children);
}

pub fn freeItems(alloc: std.mem.Allocator, items: []Item) void {
    for (items) |item| freeItem(alloc, item);
    if (items.len > 0) alloc.free(items);
}

// ---------------------------------------------------------------- tests ----

const testing = std.testing;

fn parseJson(alloc: std.mem.Allocator, source: []const u8) !struct { parsed: std.json.Parsed(std.json.Value), items: []Item } {
    const parsed = try std.json.parseFromSlice(std.json.Value, alloc, source, .{});
    errdefer parsed.deinit();
    return .{ .parsed = parsed, .items = try parse(alloc, parsed.value) };
}

test "globMatches: literals, wildcards, anchoring" {
    try testing.expect(globMatches("https://example.com/a.png", "https://example.com/a.png"));
    try testing.expect(!globMatches("https://example.com/a.png", "https://example.com/b.png"));
    try testing.expect(globMatches("*", "anything"));
    try testing.expect(globMatches("*://*.example.com/*", "https://img.example.com/cat.png"));
    try testing.expect(!globMatches("*://*.example.com/*", "https://example.org/cat.png"));
    try testing.expect(globMatches("*.png", "https://example.com/cat.png"));
    try testing.expect(!globMatches("*.png", "https://example.com/cat.jpg"));
    try testing.expect(globMatches("", ""));
    try testing.expect(!globMatches("", "x"));
}

test "hit contexts: page only when nothing more specific matched" {
    try testing.expectEqual(CTX_PAGE, (Hit{}).contexts());
    try testing.expectEqual(CTX_LINK, (Hit{ .link = "https://a/" }).contexts());
    try testing.expectEqual(CTX_IMAGE | CTX_LINK, (Hit{ .link = "https://a/", .image = "https://a/i.png" }).contexts());
    try testing.expectEqual(CTX_SELECTION, (Hit{ .has_selection = true }).contexts());
    try testing.expectEqual(CTX_EDITABLE, (Hit{ .editable = true }).contexts());
}

test "parse: defaults, types, nesting" {
    const alloc = testing.allocator;
    const source =
        \\{"items":[
        \\  {"id":"open","label":"Open link in new tab","contexts":["link"]},
        \\  {"type":"separator"},
        \\  {"id":"ext","label":"Pair Probe","contexts":["all"],"children":[
        \\    {"id":"ext-a","label":"Alpha"},
        \\    {"id":"ext-b","label":"Beta","type":"checkbox","checked":true,"enabled":false}
        \\  ]}
        \\]}
    ;
    const got = try parseJson(alloc, source);
    defer got.parsed.deinit();
    defer freeItems(alloc, got.items);

    try testing.expectEqual(@as(usize, 3), got.items.len);
    try testing.expectEqualStrings("open", got.items[0].id);
    try testing.expectEqualStrings("Open link in new tab", got.items[0].label);
    try testing.expectEqual(CTX_LINK, got.items[0].contexts);
    try testing.expectEqual(Kind.normal, got.items[0].kind);
    try testing.expect(got.items[0].enabled);

    try testing.expectEqual(Kind.separator, got.items[1].kind);

    try testing.expectEqual(CTX_ALL, got.items[2].contexts);
    try testing.expectEqual(@as(usize, 2), got.items[2].children.len);
    const beta = got.items[2].children[1];
    try testing.expectEqual(Kind.checkbox, beta.kind);
    try testing.expect(beta.checked);
    try testing.expect(!beta.enabled);
}

test "parse: an item without an id or a label is dropped, not fatal" {
    const alloc = testing.allocator;
    const got = try parseJson(alloc,
        \\{"items":[{"label":"no id"},{"id":"no-label"},7,{"id":"ok","label":"Ok"}]}
    );
    defer got.parsed.deinit();
    defer freeItems(alloc, got.items);
    try testing.expectEqual(@as(usize, 1), got.items.len);
    try testing.expectEqualStrings("ok", got.items[0].id);
}

test "matches: contexts and target globs" {
    const alloc = testing.allocator;
    const got = try parseJson(alloc,
        \\{"items":[
        \\  {"id":"page","label":"Page","contexts":["page"]},
        \\  {"id":"link","label":"Link","contexts":["link"]},
        \\  {"id":"png","label":"Png","contexts":["image"],"targetUrlGlobs":["*.png"]},
        \\  {"id":"any","label":"Any","contexts":["all"]}
        \\]}
    );
    defer got.parsed.deinit();
    defer freeItems(alloc, got.items);
    const page = got.items[0];
    const link = got.items[1];
    const png = got.items[2];
    const any = got.items[3];

    const bare = Hit{};
    try testing.expect(matches(page, bare));
    try testing.expect(!matches(link, bare));
    try testing.expect(matches(any, bare));

    const on_link = Hit{ .link = "https://example.com/doc" };
    try testing.expect(!matches(page, on_link));
    try testing.expect(matches(link, on_link));

    const on_png = Hit{ .image = "https://example.com/cat.png" };
    const on_jpg = Hit{ .image = "https://example.com/cat.jpg" };
    try testing.expect(matches(png, on_png));
    try testing.expect(!matches(png, on_jpg));
    try testing.expect(matches(any, on_jpg));
}

test "survives: a submenu whose children all filtered out is dropped" {
    const alloc = testing.allocator;
    const got = try parseJson(alloc,
        \\{"items":[
        \\  {"id":"group","label":"Group","contexts":["all"],"children":[
        \\    {"id":"only-link","label":"Only on links","contexts":["link"]}
        \\  ]},
        \\  {"id":"leaf","label":"Leaf","contexts":["all"]}
        \\]}
    );
    defer got.parsed.deinit();
    defer freeItems(alloc, got.items);
    try testing.expect(!survives(got.items[0], Hit{}));
    try testing.expect(survives(got.items[0], Hit{ .link = "https://a/" }));
    try testing.expect(survives(got.items[1], Hit{}));
}
