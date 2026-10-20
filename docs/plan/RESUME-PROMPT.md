You are the orchestrator for the NativeDesktop Rust rewrite, starting in a fresh session after a
usage pause. Your own context must stay small: you coordinate, subagents do the work.

1. Read, in this order: `docs/plan/HANDOFF.md`, `docs/plan/PROMPT.md` (the user's verbatim request and
   every dated decision; all of it still applies), `docs/plan/STATUS.md`, `docs/plan/TODO.md`,
   `docs/dev/setup.md` (machine rules: macbook low priority, g815 builds through
   `scripts/linux-gate`, never build on e1504g, disk checks). Do not read ARCHITECTURE.md or the
   research files yourself; hand subagents the sections they need.
2. Check `git status`, `git log --oneline -15` and `git worktree list` in
   `~/Developer/ecosystem/nativedesktop`, then the lane branches in the handoff table. Message the
   user one short status line before starting work.
3. Work order: land the finished lane branches (merge what passes its checks, `just clean-merged`,
   push main), then `port-gaps` into messages parity, `linux-webview` compile and gates, the
   Windows lane, the messages and youtubemusic Linux legs, then nativebrowser (M15). A port becomes
   the app's main and goes into the nix config only after its written parity checklist is fully
   checked.
4. Delegate every task to a subagent with the Agent tool (opus for design, debugging and review;
   sonnet for specified implementation), each in its own git worktree named
   `nativedesktop-wt-<lane>`, with a prompt that is self-contained: goal, files to read, what done
   means, how to verify, and the rules (paste the user's "Uncommitted work is never yours to
   discard" section verbatim, no `rm`/`mv` on variable paths, no `pkill -f`, no git
   reset/checkout/restore/stash, commit with short lowercase conventional subjects and no
   attribution trailers, never touch git identity). Run independent lanes in parallel, but at most
   two at a time that build on g815. Ask each for a final reply of at most 8 lines.
5. You merge, resolve conflicts, run the checks, push main after every merge, and keep
   `docs/plan/STATUS.md` and `docs/plan/HANDOFF.md` current, so the next session can resume the same
   way. Follow the user's output rules in ~/.claude/CLAUDE.md for every reply.
6. Every app a session modifies ends deployed on all machines (user, 2026-10-20). For messages:
   push nd main, `nix flake update nativedesktop` on messages `nd-port` (merge `nd-port-linux`
   into it first) and push, then in `~/.config/nix` `nix flake update messages`, commit only
   `flake.lock`, and switch the macbook (`darwin-rebuild`), g815 (`nixos-rebuild` on g815) and
   e1504g (built on g815, `--target-host e1504g`, never on e1504g). The same shape applies to
   every other app input in `~/.config/nix/flake.nix`.
