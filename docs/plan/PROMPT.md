# Original request (verbatim, 2026-10-13)

I think I want to fully port NativeDesktop to rust.
It should be extremely performant, with almost instant rendering, rerendering, interaction, etc.
What would be ideal is having appkit on macos, GPUI on linux and WinUI on windows (unless winUI is too crapy i'd like GPUI too)

keeping something react like and bun based would be nice, but i dont know if its truly needed since humans dont look at code anymore. Maybe having a GPUI style syntax is ideal.

All three platforms must look the same but use their own patterns automatically without modification from the app that uses nativedesktop unless it wants to override.

The rust compiler is way better than zig too, performance will be better. The target is having my e1504g laptop feel very fast and snappy even though it has terrible hardware.

This will also remove a bunch of repeated patterns from ../youtubemusic and ../messages , with matugen support for linux.

I want to build a lot of software and this would make it easier, when this is finished i would need ../youtubemusic, ../messages and ../nativebrowser ported over.

Rust has a CEF package, might be good

Having native support for cross platform and nix would be cool too to avoid repeated patterns, with a built in config system that works with home manager and generic TOML files (whatever settings format is more perfrormant and more better for us).

My only concern about a developer is storage, maybe using one of my external drives is good.


One of the reasoning is that i sometimes use mac, and on mac i like their liquid glass design. On linux i want it to match my desktop and i fell in love with the look and feel of GPUI. On windows i just use gaming so WinUI is acceptable if it fits our standards

Packaging and deploying should be easy too, preferably with some sort of centralized config system like tauri or electron builder does but cleaner and our own system.

I think GPUI also supports embedding appkit widgets but i doubt the performance is good but you can check it out: https://x.com/JaceThings/status/2107131875944968235

I want everything to work properly. Some common gaps i've seen AI agents make is window resizes, the lack of gestures, contextmenus/floating panels leaking input through, and things that embed in a window that technically is a floating thing on top like CEF doesnt react properly and doesnt feel like it's truly a part of the window/app.

I want this to be stable and be an actually feasable way to build desktop software.

A web target would be nice too lol if possible. I think GPUI supports web.

I want no true feature loss from what we already have, and the documentation needs to be reshot and remade, maybe remake it in our framework too ;)

I also want native virtualization. In messages we had to create virtualization but it was hard, especially with last rows disappearing for example, i need robust and flexible virtualization.

Use subagents for everything, make sure everything is easily configurable

One request, i want my own personal ecosystem we are making to be moved into a subfolder under Development. This would include nativedesktop, nativebrowser, nothingctl, meshtop, formalshell, messages and youtubemusic (yes, i have my own personal software ecosystem and im planning on making more :D). A file manager like files/finder would be next, you can prepare the framework a bit too.

It also needs a proper premade CLAUDE.md and agent docs in teh documentation site so that agents know the best practices and the do's and donts. A bootstrap thing would also be nice for this so that you can easily create a project with everything implemented like it should.

Smoothness is also needed and it must react accordingly with multi monitor setups. The e1504g is 60fps, the g815 is both 240hz and its other monitor 144hz and the macbook pro supports promotion (120hz). Profiling is needed for everything, and easy error reporting with logs that have robust auto cleanup so that when an issue happens we dont have to re-run the app and reproduce, we already know exactly what happened and what UI layers there are.

Auto update support is also needed for ease of use, and windows installer generation that is properlyh branded to the app the user creates.

This framework is agent first, code has to be well made and robust for ideal performance with proper guidelines and so that the agent will very likely produce good output that is robustly tested, and decent for human developers too but not as much of a priority

Also remake ndshell or improve on it so that you can simulate real use, record frames, bypass SIP and it works on all platforms. This way even animations can be polished, this would be the do it all testing framework for us. Our testing needs to be fast as fuck because often i lose hours of time because a VM gets started or there are issues with concurrent apps being developed and needing the framework.

Things like window controls and drag areas need to be a part of the framework and must integrate well with the app and OS, on hyprland no controls of course. It must be customizable so that it can look like part of the app and on macos traffic light position must be changeable using coordinates or by "embedding" it maybe so that elements can move them (similar to what arc used to do).

We also need standardized trackpad gestures so that we dont have to handrol and we can make apps feel good, maybe even haptic trackpad support like macs do for some interactivity.

Also make sure everything sstays easy to extend outside of the framework, for example if someone makes an app and wants to integrate discord rpc for instance or do whatever

An example README.md that is auto bootstrapped would be nice similar to what other react frameworks do. I want README's to be fun and a proper template to be built upon by agents similar to what we did for messages, formalmusic and formalshell

We can also expand our selection of premade components based on what i had to handroll in my gpui apps and existing nativedesktop apps, and maybe https://gpui-kit.com/ too.

Use subagents for this with proper coordination, create a todo list and my exact prompt for reference. Start in a brand new main branch and put the old one in a zig branch or something
proceed

## Decisions taken at kickoff

- Ecosystem folder: `~/Developer/ecosystem/`.
- Other repos (nativebrowser, FormalShell, nothingctl, meshtop, messages, youtubemusic) move once their Herdr panes are idle, followed by `git worktree repair`.
- Rust build output (cargo target, sccache) lives on `/Volumes/Kyan Drive/dev-cache.noindex/`.
- Old `main` renamed to `zig` (local only, origin untouched); new orphan `main` checked out at `~/Developer/ecosystem/nativedesktop`.
- 2026-10-13: the "GPUI on every platform" decision is overridden. macOS uses real AppKit controls (called from Rust through objc2, Swift only for Swift-only/SwiftUI APIs), GPUI only for custom GPU content hosted inside the AppKit tree. Windows looks native (Fluent 2 / WinUI 3 feel) via a GPUI-drawn Fluent skin, with a seam for a later WinUI backend. Linux stays GPUI with matugen. Apps write one GPUI-shaped declarative vocabulary and never see the backend.
- 2026-10-13: overrides at every layer (element, platform, mapping, theme, pattern); one `nd` crate for apps; app-defined native widgets on the same public trait the framework uses.
- 2026-10-13: docs examples are full apps in `examples/`, compiled in CI; web target is a real GPUI backend on wasm with the plain GPUI look; docs embeds show a live web tab plus harness-captured macOS, Linux and Windows screenshots and recordings.
- 2026-10-13: big structural elements use the highest-level system component (sidebar split item, NSToolbar, system caption buttons, system materials), never replicas, so new OS releases restyle apps automatically; an OS-release diff job enforces it.
- 2026-10-13: first-class `glass()` element on system materials; the zig branch's macOS command palette is ported exactly.
- 2026-10-13: update feed is static and self-hostable on any host (ecosystem uses R2 at updates.formalsnake.dev, bundle ids dev.formalsnake.*); macOS floor 14 with glass fallback below 26; git-only publishing; `zig` branch frozen at v0.5.10; signing decided at M10.
- 2026-10-13: CEF on Hyprland prefers Wayland-native (off-screen); fall back to XWayland (with `force_zero_scaling`) only where Wayland would lose functionality, e.g. Chrome-style extensions and 1Password in nativebrowser (REVIEW finding 18).
- 2026-10-13: the Windows PC drives the same 240 Hz + 144 Hz pair as g815, so the Windows pacing gate runs on real monitors (REVIEW finding 35).
- 2026-10-14 (M8): a `WebView` is an app-owned handle that owns the page; `webview(id, &page)` only shows it. Unmounting parks the page (no reload on tab switch or move between windows) and it closes when the last handle drops. Commands are handle methods; answers (`execute_javascript`, cookies, session) arrive through `FnOnce(T, &mut App)` callbacks like nd's dialogs, since nd has no executor.
- 2026-10-14 (M8): one engine seam (`nd_webview::EngineView`); WKWebView lives in nd-webview, CEF registers itself from nd-cef. The engine is `[webview] engine` (baked at build), overridable per page and by `ND_WEBVIEW_ENGINE` at build and run time. `engine = "cef"` replaces `[webview.cef] enabled`.
- 2026-10-14 (M8): requests the engine waits on (permissions, scheme requests, downloads) answer a fallback when the app drops them unanswered (WebKit's own prompt, a failed request, a cancelled download), so a page never waits on an app that had no handler.
- 2026-10-14 (M8): WebKit's inspector opens through the `_inspector` SPI (as wry does); without it the page stays inspectable from Safari. `stop` before WebKit has started the navigation waits for the start: WebKit otherwise drops it and reports the page as loading forever.
- 2026-10-14 (M8): WebView2 and WPE WebKit are not written in this lane: on Windows and Linux a web view shows the reason and every command fails with it. CEF packaging is macOS only (`nd package linux|windows` refuse `engine = "cef"`).
- 2026-10-14: a ported app becomes the default once a written parity checklist is fully checked: its nd branch merges into the app repo main and is pushed, and the nix config switches to the nd build (new dev.formalsnake.* id with data migration), rebuilt and verified on each host.
