// Wraps socket.ts's AutomationClient (reused, not forked — same framing, same
// generated RpcMethods map) so every call in the harness carries a timeout: a
// hung host (deadlock, crashed reader thread) fails a test in rpcTimeoutMs
// instead of the whole run hanging forever.
import { AutomationClient } from "./socket.ts";
import type { RpcMethodName, RpcParams, RpcResult } from "@nativedesktop/react/rpc";

export class TimedClient {
  constructor(
    private readonly inner: AutomationClient,
    private timeoutMs: number,
    private readonly stderrTail: (lines: number) => string,
  ) {}

  setTimeoutMs(ms: number): void {
    this.timeoutMs = ms;
  }

  call<M extends RpcMethodName>(
    method: M,
    ...params: RpcParams<M> extends undefined ? [] : [RpcParams<M>]
  ): Promise<RpcResult<M>> {
    const ms = this.timeoutMs;
    let timer: ReturnType<typeof setTimeout>;
    const timeout = new Promise<never>((_resolve, reject) => {
      timer = setTimeout(() => {
        reject(new Error(`${method} timed out after ${ms}ms\n--- last 40 stderr lines ---\n${this.stderrTail(40)}`));
      }, ms);
    });
    return Promise.race([this.inner.call(method, ...params), timeout]).finally(() => clearTimeout(timer)) as Promise<
      RpcResult<M>
    >;
  }

  close(): void {
    this.inner.close();
  }
}
