const std = @import("std");
const nb = @import("null_backend.zig");
const protocol = @import("protocol.zig");
const Tree = @import("tree.zig").Tree;

// schema/widgets.json is read by build.zig and injected as a build option —
// @embedFile can't reach it directly since it lives outside src/'s package path.
const schema_json = @import("build_options").schema_json;

// A dummy non-null pointer for the createWidget `app` slot (null backend ignores it).
fn dummyApp() *anyopaque {
    return @ptrFromInt(@alignOf(usize));
}

/// Synthesizes a non-default JSON value for a schema prop (type-driven).
fn synthValue(gpa: std.mem.Allocator, prop: std.json.Value) ![]const u8 {
    const t = prop.object.get("type").?.string;
    if (std.mem.eql(u8, t, "string")) return gpa.dupe(u8, "\"nd-upd\"");
    if (std.mem.eql(u8, t, "int")) return gpa.dupe(u8, "7");
    if (std.mem.eql(u8, t, "float")) return gpa.dupe(u8, "0.5");
    if (std.mem.eql(u8, t, "stringList")) return gpa.dupe(u8, "[\"a\",\"b\"]");
    if (std.mem.eql(u8, t, "bool")) {
        const d = prop.object.get("default") orelse std.json.Value{ .bool = false };
        return gpa.dupe(u8, if (d == .bool and d.bool) "false" else "true");
    }
    if (std.mem.eql(u8, t, "enum")) {
        const vals = prop.object.get("values").?.array;
        const last = vals.items[vals.items.len - 1].string;
        return std.fmt.allocPrint(gpa, "\"{s}\"", .{last});
    }
    return error.UnknownPropType; // fail loudly on a new schema prop type without a synth rule
}

fn canonAlloc(gpa: std.mem.Allocator, value: std.json.Value) ![]const u8 {
    return std.json.Stringify.valueAlloc(gpa, value, .{});
}

test "schema drives create-with-defaults for every widget" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    const parsed = try std.json.parseFromSlice(std.json.Value, gpa, schema_json, .{});
    defer parsed.deinit();
    const widgets = parsed.value.object.get("widgets").?.array;

    for (widgets.items) |w| {
        const name = w.object.get("name").?.string;
        const node = try nb.createWidget(dummyApp(), name, null);
        const sprops = w.object.get("props").?.array;
        for (sprops.items) |sp| {
            const pname = sp.object.get("name").?.string;
            const applies = sp.object.get("appliesTo").?.string;
            if (std.mem.eql(u8, applies, "meta")) continue;
            if (sp.object.get("default")) |d| {
                const expected = try canonAlloc(gpa, d);
                defer gpa.free(expected);
                try std.testing.expectEqualStrings(expected, node.props.get(pname).?);
            } else {
                try std.testing.expect(node.props.get(pname) == null);
            }
        }
    }
}

test "createAndUpdate round-trip for every widget" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    const parsed = try std.json.parseFromSlice(std.json.Value, gpa, schema_json, .{});
    defer parsed.deinit();
    const widgets = parsed.value.object.get("widgets").?.array;

    for (widgets.items) |w| {
        const name = w.object.get("name").?.string;
        const sprops = w.object.get("props").?.array;
        for (sprops.items) |sp| {
            const pname = sp.object.get("name").?.string;
            if (!std.mem.eql(u8, sp.object.get("appliesTo").?.string, "createAndUpdate")) continue;

            const node = try nb.createWidget(dummyApp(), name, null);
            const synth = try synthValue(gpa, sp);
            defer gpa.free(synth);

            const props_json = try std.fmt.allocPrint(gpa, "{{\"{s}\":{s}}}", .{ pname, synth });
            defer gpa.free(props_json);
            const props_parsed = try std.json.parseFromSlice(std.json.Value, gpa, props_json, .{});
            defer props_parsed.deinit();

            nb.applyProps(node, name, props_parsed.value);

            const expected = try canonAlloc(gpa, props_parsed.value.object.get(pname).?);
            defer gpa.free(expected);
            try std.testing.expectEqualStrings(expected, node.props.get(pname).?);
        }
    }
}

test "events connect from the schema" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    const parsed = try std.json.parseFromSlice(std.json.Value, gpa, schema_json, .{});
    defer parsed.deinit();
    const widgets = parsed.value.object.get("widgets").?.array;

    for (widgets.items) |w| {
        const name = w.object.get("name").?.string;
        const node = try nb.createWidget(dummyApp(), name, null);
        nb.connectEvents(node, name, 0);

        const sevents = w.object.get("events").?.array;
        try std.testing.expectEqual(sevents.items.len, node.events.items.len);
        for (sevents.items, node.events.items) |se, got| {
            try std.testing.expectEqualStrings(se.object.get("name").?.string, got);
        }
    }
}

test "multi-container ordering" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    const parsed = try std.json.parseFromSlice(std.json.Value, gpa, schema_json, .{});
    defer parsed.deinit();
    const widgets = parsed.value.object.get("widgets").?.array;

    for (widgets.items) |w| {
        const container = w.object.get("container").?;
        if (container == .null) continue;
        if (!std.mem.eql(u8, container.object.get("childModel").?.string, "multi")) continue;
        const name = w.object.get("name").?.string;

        // Grid is position-addressed (attachedProps carries gridRow/gridColumn):
        // ordering is not a meaningful assertion there — checked separately below.
        const is_grid = if (container.object.get("attachedProps")) |aps| blk: {
            for (aps.array.items) |ap| {
                if (std.mem.eql(u8, ap.object.get("name").?.string, "gridRow")) break :blk true;
            }
            break :blk false;
        } else false;

        const parent = try nb.createWidget(dummyApp(), name, null);
        const a = try nb.createWidget(dummyApp(), "Label", null);
        const b = try nb.createWidget(dummyApp(), "Label", null);
        const c = try nb.createWidget(dummyApp(), "Label", null);

        if (is_grid) {
            nb.appendChild(parent, name, a, .{ .grid_row = 0, .grid_column = 0 });
            nb.appendChild(parent, name, b, .{ .grid_row = 1, .grid_column = 0 });
            try std.testing.expectEqual(@as(i64, 1), b.attached.grid_row);
            try std.testing.expectEqual(@as(i64, 0), b.attached.grid_column);
            continue;
        }

        nb.appendChild(parent, name, a, .{});
        nb.appendChild(parent, name, c, .{});
        nb.insertBefore(parent, name, b, c, .{}); // -> [a, b, c]
        try std.testing.expectEqual(@as(usize, 3), parent.children.items.len);
        try std.testing.expect(parent.children.items[0] == a);
        try std.testing.expect(parent.children.items[1] == b);
        try std.testing.expect(parent.children.items[2] == c);

        nb.removeChild(parent, name, b); // -> [a, c]
        try std.testing.expectEqual(@as(usize, 2), parent.children.items.len);
        try std.testing.expect(parent.children.items[0] == a);
        try std.testing.expect(parent.children.items[1] == c);
    }
}

test "insertBefore/appendChild of an already-mounted child moves it (no duplicate)" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    const parsed = try std.json.parseFromSlice(std.json.Value, gpa, schema_json, .{});
    defer parsed.deinit();
    const widgets = parsed.value.object.get("widgets").?.array;

    for (widgets.items) |w| {
        const container = w.object.get("container").?;
        if (container == .null) continue;
        if (!std.mem.eql(u8, container.object.get("childModel").?.string, "multi")) continue;
        const name = w.object.get("name").?.string;

        const is_grid = if (container.object.get("attachedProps")) |aps| blk: {
            for (aps.array.items) |ap| {
                if (std.mem.eql(u8, ap.object.get("name").?.string, "gridRow")) break :blk true;
            }
            break :blk false;
        } else false;
        if (is_grid) continue; // ordering is not meaningful for position-addressed Grid.

        // insertBefore of an already-mounted child = move, not duplicate.
        {
            const parent = try nb.createWidget(dummyApp(), name, null);
            const a = try nb.createWidget(dummyApp(), "Label", null);
            const b = try nb.createWidget(dummyApp(), "Label", null);
            const c = try nb.createWidget(dummyApp(), "Label", null);

            nb.appendChild(parent, name, a, .{});
            nb.appendChild(parent, name, b, .{});
            nb.appendChild(parent, name, c, .{}); // -> [a, b, c]

            // Move `c` (already mounted) to before `a` -> [c, a, b].
            nb.insertBefore(parent, name, c, a, .{});
            try std.testing.expectEqual(@as(usize, 3), parent.children.items.len);
            try std.testing.expect(parent.children.items[0] == c);
            try std.testing.expect(parent.children.items[1] == a);
            try std.testing.expect(parent.children.items[2] == b);
        }

        // appendChild of an already-mounted child = move-to-end, not duplicate.
        {
            const parent = try nb.createWidget(dummyApp(), name, null);
            const a = try nb.createWidget(dummyApp(), "Label", null);
            const b = try nb.createWidget(dummyApp(), "Label", null);
            const c = try nb.createWidget(dummyApp(), "Label", null);

            nb.appendChild(parent, name, a, .{});
            nb.appendChild(parent, name, b, .{});
            nb.appendChild(parent, name, c, .{}); // -> [a, b, c]

            // Re-append `a` (already mounted) -> move to end -> [b, c, a].
            nb.appendChild(parent, name, a, .{});
            try std.testing.expectEqual(@as(usize, 3), parent.children.items.len);
            try std.testing.expect(parent.children.items[0] == b);
            try std.testing.expect(parent.children.items[1] == c);
            try std.testing.expect(parent.children.items[2] == a);
        }
    }
}

test "single-container replacement" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    const parsed = try std.json.parseFromSlice(std.json.Value, gpa, schema_json, .{});
    defer parsed.deinit();
    const widgets = parsed.value.object.get("widgets").?.array;

    for (widgets.items) |w| {
        const container = w.object.get("container").?;
        if (container == .null) continue;
        if (!std.mem.eql(u8, container.object.get("childModel").?.string, "single")) continue;
        const name = w.object.get("name").?.string;

        const parent = try nb.createWidget(dummyApp(), name, null);
        const a = try nb.createWidget(dummyApp(), "Label", null);
        const b = try nb.createWidget(dummyApp(), "Label", null);

        nb.appendChild(parent, name, a, .{});
        nb.appendChild(parent, name, b, .{});
        try std.testing.expectEqual(@as(usize, 1), parent.children.items.len);
        try std.testing.expect(parent.children.items[0] == b);
    }
}

test "tabLabel attachment" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    const tab = try nb.createWidget(dummyApp(), "TabView", null);
    const child = try nb.createWidget(dummyApp(), "Label", null);
    nb.appendChild(tab, "TabView", child, .{ .tab_label = "Form" });
    try std.testing.expectEqualStrings("Form", child.attached.tab_label.?);
}

test "unknown widget fails loudly" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    try std.testing.expectError(error.UnknownWidget, nb.createWidget(dummyApp(), "Bogus", null));
}

test "style prop rides create props (null backend round-trip)" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    const parsed = try std.json.parseFromSlice(std.json.Value, gpa, "{\"style\":{\"background\":\"#fff\",\"padding\":8}}", .{});
    defer parsed.deinit();
    const node = try nb.createWidget(dummyApp(), "Label", parsed.value);
    try std.testing.expect(node.props.get("style") != null);
}

test "style prop rides update props (null backend round-trip)" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    const node = try nb.createWidget(dummyApp(), "Label", null);
    try std.testing.expect(node.props.get("style") == null);

    const parsed = try std.json.parseFromSlice(std.json.Value, gpa, "{\"style\":{\"color\":\"#000\"}}", .{});
    defer parsed.deinit();
    nb.applyProps(node, "Label", parsed.value);
    try std.testing.expect(node.props.get("style") != null);
}

test "ListView is schema-driven with items + rowActivated event" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    const node = try nb.createWidget(dummyApp(), "ListView", null);
    nb.connectEvents(node, "ListView", 0);

    var found = false;
    for (node.events.items) |ev| {
        if (std.mem.eql(u8, ev, "rowActivated")) found = true;
    }
    try std.testing.expect(found);
}

test "update op with no widget-kind field still applies createAndUpdate props" {
    // Regression test for the tree.zig dispatch bug: React's host-config
    // `commitUpdate` never sends the widget-kind on update ops (only create
    // ops carry it), so `Tree.apply` must resolve each node's kind from the
    // retained tree, not from `op.widget`.
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();

    var t = Tree.init(gpa, dummyApp());
    defer t.deinitMeta();
    // `Tree` never tears down `nodes` itself (the real app's Tree lives for
    // the process lifetime) — free the hashmap's own storage here so this
    // test doesn't leak now that it's the first to exercise `apply`'s
    // "create" arm, which populates `nodes` (the retained widget pointers
    // themselves are owned by `nb`/`deinitAll` above, not by this map).
    defer t.nodes.deinit(gpa);

    const create_props = try std.json.parseFromSlice(std.json.Value, gpa, "{}", .{});
    defer create_props.deinit();
    var create_ops = [_]protocol.Op{
        .{ .op = "create", .id = 1, .widget = "TextInput", .props = create_props.value },
    };
    t.apply(.{ .commitId = 1, .generation = 0, .ops = &create_ops });

    const update_props = try std.json.parseFromSlice(std.json.Value, gpa, "{\"text\":\"new\"}", .{});
    defer update_props.deinit();
    var update_ops = [_]protocol.Op{
        // `widget` deliberately omitted, mirroring a real update op.
        .{ .op = "update", .id = 1, .props = update_props.value },
    };
    t.apply(.{ .commitId = 2, .generation = 0, .ops = &update_ops });

    const node = t.get(1).?;
    try std.testing.expectEqualStrings("\"new\"", node.props.get("text").?);
}
