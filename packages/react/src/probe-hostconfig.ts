import Reconciler from "react-reconciler";
import * as React from "react";

const accessed = new Set<string>();
const base: Record<string, unknown> = {
  supportsMutation: true,
  supportsPersistence: false,
  supportsHydration: false,
  isPrimaryRenderer: true,
  noTimeout: -1,
  getRootHostContext: () => ({}),
  getChildHostContext: (c: unknown) => c,
  prepareForCommit: () => null,
  resetAfterCommit: () => {},
  clearContainer: () => {},
  createInstance: () => ({}),
  createTextInstance: () => ({}),
  appendInitialChild: () => {},
  finalizeInitialChildren: () => false,
  shouldSetTextContent: () => false,
  appendChild: () => {},
  appendChildToContainer: () => {},
  insertBefore: () => {},
  insertInContainerBefore: () => {},
  removeChild: () => {},
  removeChildFromContainer: () => {},
  commitUpdate: () => {},
  commitTextUpdate: () => {},
  hideInstance: () => {},
  unhideInstance: () => {},
  hideTextInstance: () => {},
  unhideTextInstance: () => {},
  getPublicInstance: (i: unknown) => i,
  detachDeletedInstance: () => {},
  maySuspendCommit: () => false,
  preloadInstance: () => true,
  startSuspendingCommit: () => {},
  suspendInstance: () => {},
  waitForCommitToBeReady: () => null,
  resolveUpdatePriority: () => 0,
  getCurrentUpdatePriority: () => 0,
  setCurrentUpdatePriority: () => {},
  shouldAttemptEagerTransition: () => false,
  trackSchedulerEvent: () => {},
  resolveEventType: () => null,
  resolveEventTimeStamp: () => -1.1,
  requestPostPaintCallback: () => {},
  scheduleMicrotask: (fn: () => void) => queueMicrotask(fn),
  supportsMicrotasks: true,
};
const proxy = new Proxy(base, {
  get(t, k: string) { accessed.add(k); return (t as Record<string, unknown>)[k] ?? (() => {}); },
});
const r = (Reconciler as unknown as (c: unknown) => { createContainer: (...a: unknown[]) => unknown; updateContainer: (...a: unknown[]) => void })(proxy);
const container = r.createContainer({}, 0, null, false, null, "nd", () => {}, () => {}, () => {}, null);
r.updateContainer(React.createElement("box", null, React.createElement("label", null, "hi")), container, null, () => {});
setTimeout(() => { console.log([...accessed].sort().join("\n")); }, 50);
