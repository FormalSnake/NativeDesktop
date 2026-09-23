import Darwin
import Foundation

// replayd keys its ScreenCaptureKit clients by executable path. A second
// ndshot connecting cancels the first one's connection, and the request that
// was in flight on it never gets a reply: the completion handler just never
// fires. ReplayKit then reconnects, which evicts the other ndshot, so a few
// concurrent invocations livelock each other for as long as they live (seen:
// seven processes hung 50 minutes, replayd at 250% CPU). Hence one ndshot at a
// time, and a watchdog for whatever else keeps replayd from answering.

private let lockWaitSeconds = 60
private let deadlineSeconds = 15

/// Blocks until no other ndshot is talking to replayd. The lock is held until
/// the process exits (the kernel drops it on death, SIGKILL included), since
/// ReplayKit keeps its connection open for the whole process lifetime.
func acquireCaptureLock() {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("ndshot-replayd.lock").path
    let fd = open(path, O_RDWR | O_CREAT | O_CLOEXEC, 0o600)
    guard fd >= 0 else {
        eprint("ndshot: cannot open \(path) (\(String(cString: strerror(errno)))); running unserialised")
        return
    }
    let until = Date().addingTimeInterval(TimeInterval(lockWaitSeconds))
    while flock(fd, LOCK_EX | LOCK_NB) != 0 {
        if Date() > until {
            eprint("ndshot: another ndshot held \(path) for \(lockWaitSeconds)s; giving up (see `pgrep -fl ndshot`)")
            exit(5)
        }
        usleep(50_000)
    }
}

/// Exits 5 with a clear message when the command has not finished within the
/// deadline. Runs off the main thread so a stuck main run loop cannot hold it.
func armDeadline(_ command: String) {
    let timer = DispatchSource.makeTimerSource(queue: .global())
    timer.schedule(deadline: .now() + .seconds(deadlineSeconds))
    timer.setEventHandler {
        eprint(
            "ndshot: \(command) did not finish within \(deadlineSeconds)s: ScreenCaptureKit never answered "
                + "(replayd dropped the request or is wedged; `killall replayd` restarts it)")
        exit(5)
    }
    timer.resume()
    deadlineTimer = timer
}

private nonisolated(unsafe) var deadlineTimer: DispatchSourceTimer?
