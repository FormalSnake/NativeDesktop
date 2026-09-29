#if canImport(CCef)
import CCef
import Foundation

/// The browser target's DevTools protocol, over Chromium's
/// `--remote-debugging-pipe`.
///
/// `execute_dev_tools_method` (NDCefDevTools.swift) reaches one page target,
/// and a page session is refused the browser-level commands: the `Extensions`
/// domain only runs its actions for a browser target
/// (chrome/browser/devtools/chrome_devtools_session.cc), and
/// `Target.attachToBrowserTarget` needs a browser session to begin with. The
/// pipe is the one browser session that needs no listening socket, so no other
/// local process can drive the browser through it.
///
/// Framing is Chromium's default ASCIIZ mode: one JSON message per NUL.
@MainActor
enum NDCefBrowserProtocol {
    typealias Reply = (_ result: [String: Any]?, _ error: String?) -> Void

    nonisolated(unsafe) private static var readSource: DispatchSourceRead?
    private static var nextID = 1
    private static var pending: [Int: Reply] = [:]
    nonisolated(unsafe) private static var buffer = Data()

    nonisolated static var isAvailable: Bool { nd_cef_browser_pipe_write_fd() >= 0 }

    /// Starts reading replies. Called once, after `cef_initialize`.
    nonisolated static func start() {
        let fd = nd_cef_browser_pipe_read_fd()
        guard fd >= 0, readSource == nil else { return }
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: .global(qos: .userInitiated))
        source.setEventHandler {
            var chunk = [UInt8](repeating: 0, count: 64 * 1024)
            let count = read(fd, &chunk, chunk.count)
            guard count > 0 else {
                source.cancel()
                return
            }
            buffer.append(contentsOf: chunk[0..<count])
            var messages: [Data] = []
            while let end = buffer.firstIndex(of: 0) {
                messages.append(buffer[buffer.startIndex..<end])
                buffer.removeSubrange(buffer.startIndex...end)
            }
            guard !messages.isEmpty else { return }
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    for message in messages { deliver(message) }
                }
            }
        }
        readSource = source
        source.resume()
    }

    static func call(_ method: String, _ params: [String: Any] = [:], sessionID: String? = nil, reply: @escaping Reply) {
        let fd = nd_cef_browser_pipe_write_fd()
        guard fd >= 0 else {
            reply(nil, "the browser protocol pipe is not available")
            return
        }
        let id = nextID
        nextID += 1
        var message: [String: Any] = ["id": id, "method": method, "params": params]
        if let sessionID { message["sessionId"] = sessionID }
        guard var bytes = try? JSONSerialization.data(withJSONObject: message) else {
            reply(nil, "\(method): unserializable params")
            return
        }
        bytes.append(0)
        pending[id] = reply
        let written = bytes.withUnsafeBytes { raw -> Bool in
            var offset = 0
            while offset < raw.count {
                let n = write(fd, raw.baseAddress! + offset, raw.count - offset)
                if n < 0 {
                    if errno == EINTR { continue }
                    return false
                }
                offset += n
            }
            return true
        }
        if !written {
            pending.removeValue(forKey: id)
            reply(nil, "\(method): the browser protocol pipe is closed")
        }
    }

    /// Protocol events, per flat session. Only the sessions this process
    /// attached itself ever produce any.
    typealias EventHandler = (_ method: String, _ params: [String: Any]) -> Void
    private static var eventHandlers: [String: EventHandler] = [:]

    static func onEvents(sessionID: String, _ handler: EventHandler?) {
        eventHandlers[sessionID] = handler
    }

    private static func deliver(_ data: Data) {
        guard let message = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return }
        if let method = message["method"] as? String {
            if let session = message["sessionId"] as? String, let handler = eventHandlers[session] {
                handler(method, message["params"] as? [String: Any] ?? [:])
            }
            return
        }
        guard let id = message["id"] as? Int,
              let reply = pending.removeValue(forKey: id) else { return }
        if let error = message["error"] as? [String: Any] {
            reply(nil, error["message"] as? String ?? "protocol error")
        } else {
            reply(message["result"] as? [String: Any] ?? [:], nil)
        }
    }
}
#endif
