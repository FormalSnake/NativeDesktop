#!/usr/bin/env python3
# The mutter leg of crates/nd-os-linux/tests/zed_resize.rs: a headless mutter with a 1600x1000
# virtual monitor, the zed probe as a client-side decorated client (`ZED_PROBE_CSD=1`), and the
# border drags sent through mutter's RemoteDesktop D-Bus API, the only input path a headless
# mutter has. Mutter offers no capture protocol to plain clients, so this measures only how many
# frames the probe renders during the drag.
#
#   dbus-run-session -- python3 scripts/zed-resize-mutter.py <mutter> <probe> <label>
#
# Needs jeepney on PYTHONPATH. Run it inside its own dbus-run-session: the RemoteDesktop session
# is tied to the connection that created it.
import os
import subprocess
import sys
import tempfile
import threading
import time

from jeepney import DBusAddress, new_method_call
from jeepney.io.blocking import open_dbus_connection

mutter, probe, label = sys.argv[1], sys.argv[2], sys.argv[3]
runtime = tempfile.mkdtemp(prefix="zu", dir=os.environ.get("XDG_RUNTIME_DIR"))
os.chmod(runtime, 0o700)
env = dict(os.environ, XDG_RUNTIME_DIR=runtime)
for key in ("WAYLAND_DISPLAY", "DISPLAY"):
    env.pop(key, None)
log = open(os.path.join(runtime, "mutter.log"), "w")
compositor = subprocess.Popen(
    [mutter, "--headless", "--wayland", "--no-x11", "--wayland-display=wayland-zu",
     "--virtual-monitor", "1600x1000"],
    env={k: v for k, v in env.items() if k not in ("LD_LIBRARY_PATH", "VK_ICD_FILENAMES")},
    stdout=log, stderr=log)
lines = []


def collect(stream):
    for raw in stream:
        lines.append((time.monotonic(), raw.decode(errors="replace").strip()))


try:
    for _ in range(100):
        if os.path.exists(os.path.join(runtime, "wayland-zu")):
            break
        time.sleep(0.1)
    client = subprocess.Popen(
        [probe], env=dict(env, WAYLAND_DISPLAY="wayland-zu", ZED_PROBE_CSD="1"),
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    threading.Thread(target=collect, args=(client.stderr,), daemon=True).start()
    deadline = time.monotonic() + 20
    while not any(l.startswith("zed-probe: frame") for _, l in lines):
        if time.monotonic() > deadline:
            sys.exit(f"{label} mutter: the probe never drew")
        time.sleep(0.05)
    time.sleep(1)

    bus = open_dbus_connection(bus="SESSION")
    name = "org.gnome.Mutter.RemoteDesktop"
    manager = DBusAddress("/org/gnome/Mutter/RemoteDesktop", bus_name=name, interface=name)
    path = bus.send_and_get_reply(new_method_call(manager, "CreateSession")).body[0]
    session = DBusAddress(path, bus_name=name, interface=name + ".Session")
    bus.send_and_get_reply(new_method_call(session, "Start"))

    def call(method, signature, *args):
        bus.send_and_get_reply(new_method_call(session, method, signature, args))

    def pointer():
        for _, l in reversed(lines):
            if l.startswith("zed-probe: pointer"):
                x, y = l.split()[2:4]
                return float(x), float(y)
        return None

    def size():
        for _, l in reversed(lines):
            if l.startswith("zed-probe: frame"):
                w, h = l.split()[2].split("x")
                return float(w), float(h)

    def move_to(wx, wy):
        # Relative motion only: absolute motion needs a screen cast stream.
        for _ in range(5):
            here = pointer()
            if here and abs(here[0] - wx) < 1 and abs(here[1] - wy) < 1:
                return
            dx, dy = (wx - here[0], wy - here[1]) if here else (0, 0)
            call("NotifyPointerMotionRelative", "dd", dx, dy)
            time.sleep(0.15)
        raise SystemExit(f"{label} mutter: pointer did not reach {wx},{wy}: {pointer()}")

    call("NotifyPointerMotionRelative", "dd", -4000.0, -4000.0)
    call("NotifyPointerMotionRelative", "dd", 800.0, 500.0)
    time.sleep(0.3)
    w, h = size()
    before = (w, h)

    def drag(edge_x, edge_y, dx, dy):
        move_to(edge_x, edge_y)
        call("NotifyPointerButton", "ib", 0x110, True)
        time.sleep(0.1)
        t0 = time.monotonic()
        for _ in range(60):
            call("NotifyPointerMotionRelative", "dd", dx, dy)
            time.sleep(0.016)
        t1 = time.monotonic()
        call("NotifyPointerButton", "ib", 0x110, False)
        time.sleep(0.2)
        return t0, t1

    spans = [drag(w - 3, h / 2, 4.0, 0.0)]
    w2, _ = size()
    spans.append(drag(w2 / 2, h - 3, 0.0, 2.4))
    time.sleep(0.3)
    rendered = sum(1 for t, l in lines
                   if any(a <= t <= b for a, b in spans) and l.startswith("zed-probe: frame"))
    seconds = sum(b - a for a, b in spans)
    print(f"zed-resize {label} mutter: {rendered} frames rendered in {seconds:.2f} s "
          f"({rendered / seconds:.0f} Hz) for 120 steps; window {before} -> {size()}")
    client.kill()
finally:
    compositor.kill()
    compositor.wait()
