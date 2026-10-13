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
