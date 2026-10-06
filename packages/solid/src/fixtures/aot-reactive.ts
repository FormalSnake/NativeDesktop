// Built by build.test.ts and run with no register preload: the bundle has to
// carry solid-js's client build, where an effect re-runs on a signal write.
import { createRenderEffect, createRoot, createSignal, flush } from "solid-js";

const [n, setN] = createSignal(0);
const seen: number[] = [];
createRoot(() => createRenderEffect(n, (v) => void seen.push(v)));
setN(1);
flush();
console.log(`seen=${seen.join(",")}`);
