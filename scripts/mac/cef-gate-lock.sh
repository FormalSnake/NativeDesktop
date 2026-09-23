# Sourced by the mac CEF gates. One lock for the whole machine, not per
# worktree: every gate puts windows on the same screen, and a host started from
# one worktree on a cache another host holds hands its launch to that host,
# which then opens Chrome's own window in someone else's run.
#
#   cef_gate_lock     blocks until this run holds the lock, naming the holder
#   cef_gate_unlock   releases it; safe to call from an EXIT trap
#   RUN_DIR           a fresh per-run directory, removed by cef_gate_unlock
#
# mkdir is the lock because it is atomic on every filesystem macOS mounts /tmp
# on, and flock(1) does not ship with macOS.
#
# Waiters queue first-come first-served: each takes a ticket, a directory under
# the queue named by its start time and pid, and only the oldest ticket whose
# pid is alive may take the lock. A ticket left by a killed waiter is removed
# by whoever finds it.

CEF_GATE_LOCK="${ND_MAC_GATE_LOCK:-/tmp/nd-mac-cef-gate.lock}"
CEF_GATE_QUEUE="${ND_MAC_GATE_QUEUE:-${CEF_GATE_LOCK%.lock}.queue}"
CEF_GATE_HELD=""
CEF_GATE_TICKET=""
CEF_GATE_WATCH=""

# Zero-padded nanoseconds, so the names sort by age as plain strings. macOS
# date(1) has no %N.
cef_gate_now_ns() {
  perl -MTime::HiRes=time -e 'printf "%020.0f", time() * 1e9'
}

# The oldest ticket whose pid is alive, removing dead ones on the way.
cef_gate_head() {
  local t pid
  for t in $(ls -1 "$CEF_GATE_QUEUE" 2>/dev/null | sort); do
    pid="${t##*-}"
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      echo "$t"
      return
    fi
    rmdir "$CEF_GATE_QUEUE/$t" 2>/dev/null || rm -rf "$CEF_GATE_QUEUE/$t" 2>/dev/null || true
  done
}

cef_gate_lock() {
  local waited=0 holder pid head
  mkdir -p "$CEF_GATE_QUEUE" 2>/dev/null || true
  CEF_GATE_TICKET="$(cef_gate_now_ns)-$$"
  mkdir "$CEF_GATE_QUEUE/$CEF_GATE_TICKET"
  while :; do
    head="$(cef_gate_head)"
    if [ "$head" = "$CEF_GATE_TICKET" ] && mkdir "$CEF_GATE_LOCK" 2>/dev/null; then
      break
    fi
    holder="$(cat "$CEF_GATE_LOCK/owner" 2>/dev/null || true)"
    pid="${holder%% *}"
    # A holder that died without its trap (SIGKILL, a closed terminal) leaves
    # the directory behind. Its owner line is re-read before the removal, so a
    # lock taken over by somebody else in the meantime is left alone.
    if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then
      if [ "$(cat "$CEF_GATE_LOCK/owner" 2>/dev/null || true)" = "$holder" ]; then
        echo "mac CEF gate lock: taking over from a dead holder ($holder)"
        rm -rf "$CEF_GATE_LOCK"
      fi
      continue
    fi
    if [ $((waited % 30)) -eq 0 ]; then
      local ahead
      ahead="$(ls -1 "$CEF_GATE_QUEUE" 2>/dev/null | sort | awk -v me="$CEF_GATE_TICKET" '$0 < me' | wc -l | tr -d ' ')"
      echo "mac CEF gate lock: waiting ${waited}s for ${holder:-a holder that has not written its name yet}, ${ahead} ahead in the queue"
    fi
    sleep 1
    waited=$((waited + 1))
  done
  rmdir "$CEF_GATE_QUEUE/$CEF_GATE_TICKET" 2>/dev/null || true
  CEF_GATE_TICKET=""
  echo "$$ $(pwd) $0 since $(date +%H:%M:%S)" >"$CEF_GATE_LOCK/owner"
  CEF_GATE_HELD=1
  RUN_DIR="$(mktemp -d /tmp/nd-mac-cef-gate-run.XXXXXX)"
  # A short sleep per turn, not one long one: whatever captures this run's
  # output waits for every process holding it, this one included.
  (
    held=0
    while [ -d "$CEF_GATE_LOCK" ] && kill -0 $$ 2>/dev/null; do
      sleep 5
      held=$((held + 5))
      if [ "$held" -eq 600 ]; then
        echo "mac CEF gate lock: WARNING held for 10 minutes by $$ ($0), $(ls -1 "$CEF_GATE_QUEUE" 2>/dev/null | wc -l | tr -d ' ') waiting" >&2
      fi
    done
  ) </dev/null &
  CEF_GATE_WATCH=$!
}

cef_gate_unlock() {
  if [ -n "$CEF_GATE_WATCH" ]; then kill "$CEF_GATE_WATCH" 2>/dev/null || true; CEF_GATE_WATCH=""; fi
  if [ -n "$CEF_GATE_TICKET" ]; then rmdir "$CEF_GATE_QUEUE/$CEF_GATE_TICKET" 2>/dev/null || true; CEF_GATE_TICKET=""; fi
  if [ -n "${RUN_DIR:-}" ]; then rm -rf "$RUN_DIR" 2>/dev/null || true; fi
  if [ -n "$CEF_GATE_HELD" ]; then
    rm -rf "$CEF_GATE_LOCK" 2>/dev/null || true
    CEF_GATE_HELD=""
  fi
}
