import { render, useState } from "@nativedesktop/react";

// A REMOTE terminal: the <terminal remote> widget opens the ndremote byte-plane
// transport (host-side, in the ND process) instead of a local PTY, attaches a
// session on a Canary daemon, and renders the streamed output. No terminal
// bytes ever cross NDP/React — the app only sees the effect + connection-state
// events (title/bell/exit/connectionState), which it surfaces into labels here.
//
// Connection target comes from the environment so a fake server on an ephemeral
// port can drive it in CI (scripts/remote-terminal-fake-server.ts).
const HOST = process.env.ND_REMOTE_HOST ?? "127.0.0.1";
const PORT = Number(process.env.ND_REMOTE_PORT ?? "4618");
const SESSION = process.env.ND_REMOTE_SESSION ?? "sess-demo";
const TICKET = process.env.ND_REMOTE_TICKET ?? "ticket-demo";
// WP polish-1 deliverable 3 proof (scripts/remote-terminal.sh): unset by
// default, so this example still exercises the "no fontFamily" fallback path.
const FONT_FAMILY = process.env.ND_TERM_FONT_FAMILY;

// nd_rt_state order (include/ndremote.h).
const STATE_NAMES = ["connecting", "authed", "attached", "reconnecting", "failed", "closed"];

function App(): React.ReactNode {
  const [conn, setConn] = useState("connecting");
  const [title, setTitle] = useState("");
  const [bells, setBells] = useState(0);
  const [exitCode, setExitCode] = useState<number | null>(null);

  return (
    <window title="Remote Terminal" defaultWidth={860} defaultHeight={560}>
      <toolbarview>
        <headerbar title="Remote Terminal" testID="chrome" />
        <box
          orientation="vertical"
          spacing={10}
          style={{
            background: "#0e0e12",
            padding: 14,
            vexpand: true,
          }}
        >
          <box orientation="horizontal" spacing={18}>
            <label testID="conn-state" text={`conn: ${conn}`} />
            <label testID="term-title" text={`title: ${title}`} />
            <label testID="term-bell" text={`bells: ${bells}`} />
            <label testID="term-exited" text={exitCode === null ? "exit: -" : `exit: ${exitCode}`} />
          </box>
          <terminal
            remote
            host={HOST}
            port={PORT}
            sessionId={SESSION}
            ticket={TICKET}
            cols={80}
            rows={24}
            fontSize={13}
            fontFamily={FONT_FAMILY}
            testID="remote-term"
            onConnectionState={(e) => {
              const s = (e.data as { state: number }).state;
              setConn(STATE_NAMES[s] ?? String(s));
            }}
            onTitleChanged={(e) => setTitle(e.text)}
            onBell={() => setBells((n) => n + 1)}
            onExited={(e) => setExitCode((e.data as { code: number }).code)}
            style={{ hexpand: true, vexpand: true }}
          />
        </box>
      </toolbarview>
    </window>
  );
}

await render(<App />);
