// Run by renderer.test.tsx in a child process: a render error no app
// boundary catches has to exit it.
import { createSignal, Show } from "solid-js";
import { Batch, NodeRegistry, setSession, setUnhandledErrorPolicy } from "@nativedesktop/react/core";
import { render } from "../renderer.ts";

const batch = new Batch();
setSession({
  ndp: { sendRuntimeError() {} } as never,
  registry: new NodeRegistry(),
  batch,
  commit: () => void batch.drain(),
  flush() {},
});
setUnhandledErrorPolicy({ uncaughtException: "report", unhandledRejection: "report" });
const [armed, setArmed] = createSignal(false);
const Boom = (): never => {
  throw new Error("uncaught-render");
};
await render(() => (
  <window>
    <Show when={armed()}>
      <Boom />
    </Show>
  </window>
));
setArmed(true);
setTimeout(() => console.log("STILL_ALIVE"), 200);
