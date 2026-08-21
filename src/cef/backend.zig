// Entry point for the CEF <webview> engine, and the seam that keeps it out of
// builds with no CEF distribution to translate headers against.
//
// `impl` is a comptime choice between two whole files: the untaken branch of a
// comptime `if` is never analyzed, so `engine.zig`'s `@cImport` of the capi
// headers never runs on a build that has no dist (or on macOS, where windowed
// embedding is the other lane's problem and cef_types_linux.h does not exist).
const builtin = @import("builtin");
const build_options = @import("build_options");

pub const compiled_in = build_options.cef_engine and builtin.os.tag == .linux;

const impl = if (compiled_in) @import("engine.zig") else @import("absent.zig");

pub const Info = @import("types.zig").Info;
pub const EmitFn = @import("types.zig").EmitFn;

pub const earlyExecuteProcess = impl.earlyExecuteProcess;
pub const pinDisplayBackend = impl.pinDisplayBackend;
pub const shutdown = impl.shutdown;
pub const setOnInitialized = impl.setOnInitialized;
pub const create = impl.create;
pub const isReal = impl.isReal;
pub const setUrl = impl.setUrl;
pub const command = impl.command;
pub const connectEvents = impl.connectEvents;
pub const info = impl.info;
pub const evalStart = impl.evalStart;
pub const evalPoll = impl.evalPoll;
pub const evalRelease = impl.evalRelease;
pub const pageText = impl.pageText;
pub const registerScheme = impl.registerScheme;
pub const setContextMenuMode = impl.setContextMenuMode;
pub const EvalState = impl.EvalState;
