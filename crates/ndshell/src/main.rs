//! `ndshell grant <binary>`, `ndshell preflight`, `ndshell lock` and `ndshell agent`; see the
//! library modules.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("lock") => lock(&args[1..]),
        Some("agent") => agent(&args[1..]),
        _ => run(&args),
    };
    if let Err(message) = result {
        eprintln!("ndshell: {message}");
        std::process::exit(1);
    }
}

const USAGE: &str = "usage: ndshell grant <binary> | ndshell preflight | ndshell lock [status | -- <command>...] \
                     | ndshell agent [install | uninstall | stop | run | ping | call <json>]";

/// `ping` and `call` talk to an agent (on this machine, or forwarded with `ssh -L`); the rest
/// manage it on Windows.
fn agent(args: &[String]) -> Result<(), String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["ping"] => {
            let reply = ndshell::agent::Client::connect()?.call(serde_json::json!({ "cmd": "ping" }))?;
            println!("{reply}");
            Ok(())
        }
        ["call", json] => {
            let request = serde_json::from_str(json).map_err(|e| format!("request: {e}"))?;
            println!("{}", ndshell::agent::Client::connect()?.call(request)?);
            Ok(())
        }
        #[cfg(windows)]
        ["install"] => ndshell::agent::install::install().map(|done| println!("ndshell: {done}")),
        #[cfg(windows)]
        ["uninstall"] => ndshell::agent::install::uninstall(),
        #[cfg(windows)]
        ["stop"] => ndshell::agent::install::stop(),
        #[cfg(windows)]
        ["run"] => ndshell::agent::server::run(),
        _ => Err(USAGE.into()),
    }
}

/// `ndshell lock status` prints the holder; `ndshell lock -- <command>` runs the command holding
/// the machine-wide real-input lock, queueing behind the current holder.
fn lock(args: &[String]) -> Result<(), String> {
    match args {
        [status] if status == "status" => {
            let holder = ndshell::lock::holder();
            println!("{}", if holder.is_empty() { "ndshell: the real-input lock is free".into() } else { holder });
            Ok(())
        }
        [dashes, program, rest @ ..] if dashes == "--" => {
            let purpose = std::iter::once(program).chain(rest).cloned().collect::<Vec<_>>().join(" ");
            let _lock = ndshell::lock::acquire(&purpose).map_err(|e| format!("lock: {e}"))?;
            let status =
                std::process::Command::new(program).args(rest).status().map_err(|e| format!("{program}: {e}"))?;
            if status.success() { Ok(()) } else { Err(format!("{program} exited with {status}")) }
        }
        _ => Err(USAGE.into()),
    }
}

#[cfg(target_os = "macos")]
fn run(args: &[String]) -> Result<(), String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["grant", binary] => {
            let (path, identifier) = ndshell::grant::grant(std::path::Path::new(binary))?;
            println!(
                "ndshell: Screen Recording, Accessibility and PostEvent granted to {path} (identifier \"{identifier}\")"
            );
            Ok(())
        }
        ["preflight"] => {
            ndshell::preflight::real_input()?;
            println!("ndshell: ready for real input");
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// `ndshell drag <x> <y> <dx> <dy> <steps> <step_ms> <output_w> <output_h>` drags with a virtual
/// pointer in the session `WAYLAND_DISPLAY` names, printing `drag motion t_us=.. x=.. y=..` per
/// motion on the monotonic clock, so an app's present times can be set against the pointer.
#[cfg(target_os = "linux")]
fn run(args: &[String]) -> Result<(), String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if let ["shot", rest @ ..] = args.as_slice() {
        return shot(rest);
    }
    let ["drag", rest @ ..] = args.as_slice() else {
        return Err(format!(
            "{USAGE} | ndshell drag <x> <y> <dx> <dy> <steps> <step_ms> <output_w> <output_h> \
             | ndshell shot <out.png> <seconds> <scale> -- <command>..."
        ));
    };
    let n: Vec<f64> =
        rest.iter().map(|a| a.parse::<f64>().map_err(|e| format!("{a}: {e}"))).collect::<Result<_, _>>()?;
    let [x, y, dx, dy, steps, step_ms, w, h] = n[..] else { return Err("drag takes eight numbers".into()) };
    let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(|_| "XDG_RUNTIME_DIR is not set")?;
    let display = std::env::var("WAYLAND_DISPLAY").map_err(|_| "WAYLAND_DISPLAY is not set")?;
    let socket = std::path::Path::new(&runtime).join(display);
    let mut input = ndshell::wayland::VirtualInput::connect_socket(&socket, (w as u32, h as u32))?;
    let now_us = || {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        t.tv_sec as u64 * 1_000_000 + t.tv_nsec as u64 / 1000
    };
    input.move_to(x, y);
    input.sync()?;
    std::thread::sleep(std::time::Duration::from_millis(80));
    input.button(true);
    input.sync()?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    let steps = steps as u32;
    for i in 1..=steps {
        let t = f64::from(i) / f64::from(steps);
        let (px, py) = (x + dx * t, y + dy * t);
        input.move_to(px, py);
        println!("drag motion t_us={} x={px:.1} y={py:.1}", now_us());
        std::thread::sleep(std::time::Duration::from_secs_f64(step_ms / 1000.));
    }
    input.sync()?;
    std::thread::sleep(std::time::Duration::from_millis(300));
    input.button(false);
    input.sync()
}

/// `ndshell shot <out.png> <seconds> <scale> -- <command>...` runs the command in a private
/// headless Hyprland at `scale`, waits `seconds`, and writes its first window (the whole output
/// when it opened none) to `out.png`, so nd and real GTK apps are captured the same way. The
/// window the compositor focuses draws as active; a later window takes the focus from it.
#[cfg(target_os = "linux")]
fn shot(args: &[&str]) -> Result<(), String> {
    let [out, seconds, scale, "--", program, rest @ ..] = args else {
        return Err("usage: ndshell shot <out.png> <seconds> <scale> -- <command>...".into());
    };
    let seconds: f64 = seconds.parse().map_err(|e| format!("{seconds}: {e}"))?;
    let scale: f64 = scale.parse().map_err(|e| format!("{scale}: {e}"))?;
    // A 1280x800 layout at any scale, room for a GNOME app's default window size.
    let options = ndshell::hyprland::Options {
        width: (1280. * scale).round() as u32,
        height: (800. * scale).round() as u32,
        ..ndshell::hyprland::Options::default()
    };
    let extra_config = format!(
        "hl.monitor({{ output = \"{}\", mode = \"{}x{}@{}\", position = \"0x0\", scale = \"{scale}\" }})",
        ndshell::hyprland::OUTPUT,
        options.width,
        options.height,
        options.refresh
    );
    let hyprland = ndshell::hyprland::Hyprland::start(ndshell::hyprland::Options { extra_config, ..options })?;
    // A seat without a keyboard never gives a window keyboard focus, so every app would draw its
    // inactive (backdrop) look; the virtual keyboard stands in for the one a desktop has.
    let _keyboard = ndshell::wayland::VirtualInput::connect(&hyprland)?;
    let mut child = hyprland.command(program).args(rest).spawn().map_err(|e| format!("{program}: {e}"))?;
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    let frame = ndshell::wayland::screenshot(&hyprland);
    let client = hyprland.clients().ok().and_then(|c| c.into_iter().next());
    let _ = child.kill();
    let _ = child.wait();
    let frame = frame?;
    let (mut x, mut y, mut w, mut h) = (0, 0, frame.width, frame.height);
    if let Some(c) = client {
        let px = |v: i32| (f64::from(v) * scale).round().max(0.) as u32;
        x = px(c.at.0).min(frame.width);
        y = px(c.at.1).min(frame.height);
        w = px(c.size.0).min(frame.width - x);
        h = px(c.size.1).min(frame.height - y);
    }
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        for col in x..x + w {
            let i = (row * frame.stride + col * 4) as usize;
            let p = &frame.data[i..i + 4];
            let (r, b) = if frame.red_first { (p[0], p[2]) } else { (p[2], p[0]) };
            rgba.extend_from_slice(&[r, p[1], b, 255]);
        }
    }
    let file = std::fs::File::create(out).map_err(|e| format!("{out}: {e}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut wr| wr.write_image_data(&rgba)).map_err(|e| format!("{out}: {e}"))?;
    println!("ndshell: wrote {out} ({w}x{h})");
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn run(_args: &[String]) -> Result<(), String> {
    Err(format!("ndshell has only the lock and the agent on this OS; {USAGE}"))
}
