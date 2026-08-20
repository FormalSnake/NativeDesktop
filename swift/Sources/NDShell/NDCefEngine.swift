#if canImport(CCef)
import AppKit
import CCef
import Foundation

/// Process-level lifecycle for the Chromium (CEF) `<webview>` engine: finding
/// the distribution, loading it at runtime, initializing the library, and
/// owning the main loop. Per-view work lives in NDCefWebView.swift.
///
/// The engine is opt-in (`ND_WEBVIEW_ENGINE=chromium`) and never linked: the
/// framework is dlopened through CCef, so a system-engine build carries no
/// Chromium bytes and this whole file is inert.
///
/// macOS pins three things the spec settles and this file must not drift from:
/// CEF owns the message loop (`cef_run_message_loop` replaces `[NSApp run]`),
/// NSApplication must conform to CefAppProtocol, and every child process comes
/// from a helper `.app` inside `Contents/Frameworks` rather than a re-exec of
/// this binary.
enum NDCefRuntime {
    /// Set by `prepare()`; the engine only exists for the rest of the process
    /// once this is true.
    nonisolated(unsafe) private(set) static var isActive = false
    nonisolated(unsafe) private static var paths: NDCefPaths?

    static var isRequested: Bool {
        ProcessInfo.processInfo.environment["ND_WEBVIEW_ENGINE"] == "chromium"
    }

    /// Whether one view gets the Chromium surface, given its create-only
    /// `engine` prop.
    ///
    /// Selection has two layers. The process-level one is the env handshake
    /// (`nd dev` resolves `webview.engine.mac` and re-exports it), and it has
    /// to be the default for every view because CEF is initialized before any
    /// view exists. The prop is the per-view layer and can name chromium on
    /// top of that. It cannot name a view back OUT of a chromium process:
    /// React omits props sitting at their default, so "system" and "unset"
    /// arrive identically by the time the generated create arm has run.
    static func wants(_ prop: String) -> Bool {
        isActive && (prop == "chromium" || isRequested)
    }

    /// Resolves the distribution and loads it. Must run before anything
    /// touches `NSApplication.shared`, because a CEF process needs the
    /// CefAppProtocol subclass to be the one that gets created.
    ///
    /// Every failure here is a fallback, not a crash: the widget keeps working
    /// on the system engine, which is what `<webview>` promises when the
    /// requested engine is unavailable.
    static func prepare() -> Bool {
        guard isRequested else { return false }
        guard let resolved = NDCefPaths.resolve() else {
            ndCefWarn("ND_WEBVIEW_ENGINE=chromium: no CEF distribution found (set ND_CEF_ROOT); using the system engine")
            return false
        }
        guard nd_cef_load(resolved.frameworkBinary) != 0 else {
            let reason = nd_cef_load_error().map { String(cString: $0) } ?? "unknown"
            ndCefWarn("ND_WEBVIEW_ENGINE=chromium: \(reason); using the system engine")
            return false
        }
        // First call after the load, before any other entry point: the loaded
        // framework's struct layout has to match the headers this binary was
        // compiled against or every handler below is misaligned.
        let version = nd_cef_compiled_api_version()
        let compiled = nd_cef_compiled_api_hash().map { String(cString: $0) } ?? ""
        let loaded = nd_cef_api_hash(version, 0).map { String(cString: $0) } ?? ""
        guard !loaded.isEmpty, loaded == compiled else {
            ndCefWarn("ND_WEBVIEW_ENGINE=chromium: API \(version) hash mismatch (framework \(loaded), headers \(compiled)); using the system engine")
            return false
        }
        paths = resolved
        isActive = true
        return true
    }

    /// `cef_initialize`, after the core runtime is up. Returns false when CEF
    /// refused to start, which drops the process back to `[NSApp run]`.
    static func initialize() -> Bool {
        guard isActive, let paths else { return false }
        var settings = cef_settings_t()
        settings.size = MemoryLayout<cef_settings_t>.size
        // The macOS sandbox needs cef_sandbox.a linked into the host, which is
        // exactly the "never link CEF" line this integration holds. Chromium's
        // own per-process sandboxes still apply.
        settings.no_sandbox = 1
        settings.multi_threaded_message_loop = 0
        settings.external_message_pump = 0
        settings.windowless_rendering_enabled = 0
        settings.log_severity = LOGSEVERITY_WARNING

        var owned: [UnsafeMutablePointer<cef_string_t>] = []
        func set(_ keyPath: WritableKeyPath<cef_settings_t, cef_string_t>, _ value: String) {
            guard !value.isEmpty else { return }
            let slot = UnsafeMutablePointer<cef_string_t>.allocate(capacity: 1)
            slot.initialize(to: cef_string_t())
            ndCefSetString(value, slot)
            settings[keyPath: keyPath] = slot.pointee
            owned.append(slot)
        }
        set(\.browser_subprocess_path, paths.helperExecutable)
        set(\.framework_dir_path, paths.frameworkDirectory)
        set(\.main_bundle_path, paths.mainBundle)
        set(\.root_cache_path, paths.rootCache)
        defer {
            for slot in owned {
                nd_cef_string_clear(slot)
                slot.deinitialize(count: 1)
                slot.deallocate()
            }
        }

        var args = cef_main_args_t(argc: CommandLine.argc, argv: CommandLine.unsafeArgv)
        guard nd_cef_initialize(&args, &settings, application, nil) != 0 else {
            ndCefWarn("cef_initialize failed; using the system engine")
            isActive = false
            return false
        }
        FileHandle.standardError.write("ND_WEBVIEW_ENGINE chromium (\(paths.frameworkDirectory))\n".data(using: .utf8)!)
        return true
    }

    /// The process-level `cef_app_t`, alive for as long as CEF is. Its one job
    /// is the command line: Chromium's popup blocker swallows a `window.open`
    /// that had no user gesture BEFORE `on_before_popup` runs, which would
    /// silently drop the `newWindow` event the WKWebView surface fires for the
    /// same page. Nothing here may open a window, so the blocker only costs us
    /// the event.
    nonisolated(unsafe) private static let application: UnsafeMutablePointer<cef_app_t>? = {
        guard let raw = nd_cef_ref_alloc(MemoryLayout<cef_app_t>.size, nil, nil) else { return nil }
        let app = raw.assumingMemoryBound(to: cef_app_t.self)
        app.pointee.on_before_command_line_processing = { _, processType, commandLine in
            defer { nd_cef_ref_release(commandLine) }
            // Browser process only: the renderer's own command line is
            // Chromium's to build.
            guard ndCefString(processType).isEmpty, let commandLine else { return }
            var name = cef_string_t()
            ndCefSetString("disable-popup-blocking", &name)
            defer { nd_cef_string_clear(&name) }
            commandLine.pointee.append_switch?(commandLine, &name)
        }
        return app
    }()

    /// Replaces `NSApplication.run()`. CEF's mac pump drives `[NSApp run]`
    /// itself, so the app delegate, the menu bar and terminate all behave as
    /// they do on the system engine.
    static func runMessageLoop() {
        nd_cef_run_message_loop()
        nd_cef_shutdown()
    }
}

/// The five paths CEF needs on macOS, resolved once at startup.
struct NDCefPaths {
    let frameworkDirectory: String
    let frameworkBinary: String
    let helperExecutable: String
    let mainBundle: String
    let rootCache: String

    private static let frameworkName = "Chromium Embedded Framework.framework"
    private static let frameworkBinaryName = "Chromium Embedded Framework"

    /// Resolution order per the spec: ND_CEF_ROOT, then the app bundle, then
    /// the dev cache. The helper bundle is always the app's own, because CEF
    /// derives the other four child bundles by suffixing its name and nothing
    /// outside the bundle can stand in for them.
    static func resolve() -> NDCefPaths? {
        let bundle = Bundle.main.bundleURL
        guard bundle.pathExtension == "app" else {
            ndCefWarn("ND_WEBVIEW_ENGINE=chromium needs a bundled host (scripts/mac/dev-cef-bundle.sh); running bare")
            return nil
        }
        let executableName = Bundle.main.executableURL?.deletingPathExtension().lastPathComponent ?? "NDShell"
        let frameworksDirectory = bundle.appendingPathComponent("Contents/Frameworks")
        let helper = frameworksDirectory
            .appendingPathComponent("\(executableName) Helper.app/Contents/MacOS/\(executableName) Helper")
        guard FileManager.default.isExecutableFile(atPath: helper.path) else {
            ndCefWarn("ND_WEBVIEW_ENGINE=chromium: no helper app at \(helper.path)")
            return nil
        }

        let environment = ProcessInfo.processInfo.environment
        var candidates: [URL] = []
        if let root = environment["ND_CEF_ROOT"], !root.isEmpty {
            let url = URL(fileURLWithPath: root)
            candidates.append(url.appendingPathComponent("Release/\(frameworkName)"))
            candidates.append(url.appendingPathComponent(frameworkName))
        }
        candidates.append(frameworksDirectory.appendingPathComponent(frameworkName))
        candidates.append(
            URL(fileURLWithPath: NSHomeDirectory())
                .appendingPathComponent(".cache/nativedesktop/cef/\(ndCefVersion)/Release/\(frameworkName)")
        )

        for directory in candidates {
            let binary = directory.appendingPathComponent(frameworkBinaryName)
            guard FileManager.default.fileExists(atPath: binary.path) else { continue }
            return NDCefPaths(
                frameworkDirectory: directory.path,
                frameworkBinary: binary.path,
                helperExecutable: helper.path,
                mainBundle: bundle.path,
                rootCache: cacheDirectory(for: executableName)
            )
        }
        return nil
    }

    /// One root cache for the process; per-profile caches hang off it in M2.
    private static func cacheDirectory(for executableName: String) -> String {
        if let override = ProcessInfo.processInfo.environment["ND_CEF_CACHE"], !override.isEmpty {
            return override
        }
        return URL(fileURLWithPath: NSHomeDirectory())
            .appendingPathComponent("Library/Caches/nativedesktop/cef/\(executableName)")
            .path
    }
}

/// The distribution the build was pinned to, mirrored from swift/Package.swift.
let ndCefVersion = "151.3.23-macosarm64"

// MARK: - NSApplication

/// CEF's mac message pump asks NSApp whether it is inside `sendEvent:` before
/// it will let a nested loop exit, so the application object has to answer
/// this protocol. Chromium looks it up by name through the ObjC runtime, which
/// is why the Swift declaration is renamed onto CEF's own.
@objc(CefAppProtocol) protocol NDCefAppProtocol {
    func isHandlingSendEvent() -> Bool
    func setHandlingSendEvent(_ handlingSendEvent: Bool)
}

/// `NSApplication.shared` for a CEF process. Nothing may create the shared
/// application before this class does, or NSApp is a plain NSApplication and
/// CEF aborts on the protocol check.
final class NDCefApplication: NSApplication, NDCefAppProtocol {
    nonisolated(unsafe) private var handlingSendEvent = false

    nonisolated func isHandlingSendEvent() -> Bool { handlingSendEvent }

    nonisolated func setHandlingSendEvent(_ handlingSendEvent: Bool) {
        self.handlingSendEvent = handlingSendEvent
    }

    /// The scoped flag CEF's CefScopedSendingEvent sets: saved and restored
    /// rather than cleared, because event dispatch nests.
    override func sendEvent(_ event: NSEvent) {
        let previous = handlingSendEvent
        handlingSendEvent = true
        super.sendEvent(event)
        handlingSendEvent = previous
    }
}

// MARK: - Shared helpers

func ndCefWarn(_ message: String) {
    FileHandle.standardError.write("ND_WARN \(message)\n".data(using: .utf8)!)
}

/// Fills a `cef_string_t` from a Swift string. The library owns the copy after
/// this, and `nd_cef_string_clear` is what releases it.
func ndCefSetString(_ value: String, _ out: UnsafeMutablePointer<cef_string_t>) {
    var utf8 = Array(value.utf8)
    utf8.withUnsafeBufferPointer { buffer in
        guard let base = buffer.baseAddress else { return }
        base.withMemoryRebound(to: CChar.self, capacity: buffer.count) { chars in
            _ = nd_cef_string_set(chars, buffer.count, out)
        }
    }
}

/// Reads a `cef_string_t` CEF handed us. The storage stays CEF's.
func ndCefString(_ value: UnsafePointer<cef_string_t>?) -> String {
    guard let value, let characters = value.pointee.str, value.pointee.length > 0 else { return "" }
    return String(decoding: UnsafeBufferPointer(start: characters, count: value.pointee.length), as: UTF16.self)
}
#endif
