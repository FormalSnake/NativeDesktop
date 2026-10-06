import { render } from "@nativedesktop/solid";
import { Errored, Show, createSignal } from "solid-js";

function Thrower(): never {
  throw new Error("render-throw");
}

function App() {
  const [count, setCount] = createSignal(0);
  const [armed, setArmed] = createSignal(false);
  return (
    <window title="NativeDesktop Error Policy" defaultWidth={480} defaultHeight={360}>
      <box orientation="vertical" spacing={8}>
        <label testID="counter-label" text={`Count: ${count()}`} />
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
            // Outside any Solid computation: an uncaughtException, fatal by default.
            setTimeout(() => {
              throw new Error("sync-throw");
            }, 0);
          }}
        />
        {/* The caught throw is reported non-fatal and the app keeps running;
            the same throw with no boundary around it would exit. */}
        <Errored fallback={(error) => <label testID="boundary-fallback" text={`caught: ${(error() as Error).message}`} />}>
          <Show when={armed()} fallback={<label testID="boundary-content" text="boundary content ok" />}>
            <Thrower />
          </Show>
        </Errored>
      </box>
    </window>
  );
}

await render(() => <App />);
