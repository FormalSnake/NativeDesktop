import { render, useState } from "@nativedesktop/react";
// Class components hold no hook state, so the dev-react re-export rule
// (hooks come from @nativedesktop/react) does not apply to the base class.
import { Component, type ReactNode } from "react";

class Boundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };
  static getDerivedStateFromError(error: Error): { error: Error } {
    return { error };
  }
  render(): ReactNode {
    if (this.state.error) {
      return <label testID="boundary-fallback" text={`caught: ${this.state.error.message}`} />;
    }
    return this.props.children;
  }
}

function Thrower({ armed }: { armed: boolean }): ReactNode {
  if (armed) throw new Error("render-throw");
  return <label testID="boundary-content" text="boundary content ok" />;
}

function App(): ReactNode {
  const [count, setCount] = useState(0);
  const [armed, setArmed] = useState(false);
  return (
    <window title="NativeDesktop Error Policy" defaultWidth={480} defaultHeight={360}>
      <box orientation="vertical" spacing={8}>
        <label testID="counter-label" text={`Count: ${count}`} />
        <button testID="bump" label="Bump" onClick={() => setCount((c) => c + 1)} />
        <button
          testID="reject-async"
          label="Reject a promise"
          onClick={() => {
            // Fire-and-forget rejection: default policy reports and survives.
            void Promise.reject(new Error("async-reject"));
          }}
        />
        <button testID="throw-caught" label="Throw in render (caught)" onClick={() => setArmed(true)} />
        <button
          testID="throw-sync"
          label="Throw sync"
          onClick={() => {
            // Outside any React callback: an uncaughtException, fatal by default.
            setTimeout(() => {
              throw new Error("sync-throw");
            }, 0);
          }}
        />
        <Boundary>
          <Thrower armed={armed} />
        </Boundary>
      </box>
    </window>
  );
}

await render(<App />);
