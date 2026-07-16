#!/usr/bin/env bun
// scripts/remote-terminal-fake-server.ts — a self-contained Canary byte-plane
// server (docs/protocol.md §2) for driving the <terminal remote> widget. NO
// dependency on the canary repo: the u32-len|u8-type framing is hand-rolled
// here. On ATTACH it streams a scripted sequence that exercises every remote
// event: a banner, an OSC-2 title, a bell, a FLAG_RESET snapshot (with its own
// title so onTitleChanged proves the post-reset bytes were fed), then EXIT.
//
// Usage: bun scripts/remote-terminal-fake-server.ts [port]  (0/absent = ephemeral)
// Prints `FAKE_SERVER_LISTENING port=<port>` once bound.

const T = {
  PING: 0x00, PONG: 0x01, AUTH: 0x02, AUTH_OK: 0x03, AUTH_ERR: 0x04,
  ATTACH: 0x05, ATTACHED: 0x06, DETACH: 0x07, OUTPUT: 0x10, INPUT: 0x11,
  RESIZE: 0x12, RESIZED: 0x13, ACK: 0x14, EXIT: 0x15,
} as const;
const FLAG_RESET = 0x01;
const enc = new TextEncoder();

function frame(type: number, body: Uint8Array): Uint8Array {
  const out = new Uint8Array(5 + body.length);
  new DataView(out.buffer).setUint32(0, 1 + body.length, true);
  out[4] = type;
  out.set(body, 5);
  return out;
}
const jsonBody = (o: unknown) => enc.encode(JSON.stringify(o));

function outputBody(channel: number, seq: number, flags: number, data: Uint8Array): Uint8Array {
  const b = new Uint8Array(13 + data.length);
  const dv = new DataView(b.buffer);
  dv.setUint32(0, channel, true);
  dv.setBigUint64(4, BigInt(seq), true);
  dv.setUint8(12, flags);
  b.set(data, 13);
  return b;
}
function u32u16u16(channel: number, cols: number, rows: number): Uint8Array {
  const b = new Uint8Array(8);
  const dv = new DataView(b.buffer);
  dv.setUint32(0, channel, true);
  dv.setUint16(4, cols, true);
  dv.setUint16(6, rows, true);
  return b;
}
function exitBody(channel: number, code: number): Uint8Array {
  const b = new Uint8Array(8);
  const dv = new DataView(b.buffer);
  dv.setUint32(0, channel, true);
  dv.setInt32(4, code, true);
  return b;
}

interface ConnState { buf: Uint8Array; seq: number }
const conns = new WeakMap<object, ConnState>();

function drainFrames(st: ConnState, chunk: Uint8Array): Array<{ type: number; body: Uint8Array }> {
  const merged = new Uint8Array(st.buf.length + chunk.length);
  merged.set(st.buf, 0);
  merged.set(chunk, st.buf.length);
  st.buf = merged;
  const out: Array<{ type: number; body: Uint8Array }> = [];
  let off = 0;
  while (st.buf.length - off >= 4) {
    const len = new DataView(st.buf.buffer, st.buf.byteOffset + off, 4).getUint32(0, true);
    if (st.buf.length - off < 4 + len) break;
    out.push({ type: st.buf[off + 4]!, body: st.buf.subarray(off + 5, off + 4 + len) });
    off += 4 + len;
  }
  st.buf = st.buf.subarray(off);
  return out;
}

function streamSession(sock: { write(d: Uint8Array): number }, st: ConnState, cols: number, rows: number): void {
  const send = (bytes: Uint8Array, flags = 0) => {
    st.seq += bytes.length;
    sock.write(frame(T.OUTPUT, outputBody(1, st.seq, flags, bytes)));
  };
  setTimeout(() => send(enc.encode("Canary remote terminal ready\r\n")), 150);
  setTimeout(() => send(enc.encode("\x1b]2;remote-shell\x07")), 350); // OSC-2 title
  setTimeout(() => send(enc.encode("\x07")), 500); // bell
  // FLAG_RESET snapshot: clears the grid, then feeds new content + a new title.
  setTimeout(() => send(enc.encode("\x1b]2;snapshot-ready\x07SNAPSHOT-RELOADED\r\n"), FLAG_RESET), 750);
  setTimeout(() => sock.write(frame(T.EXIT, exitBody(1, 0))), 1000);
  void cols; void rows;
}

const port = Number(process.argv[2] ?? process.env.ND_FAKE_PORT ?? 0);
const server = Bun.listen<undefined>({
  hostname: "127.0.0.1",
  port,
  socket: {
    open(sock) {
      conns.set(sock, { buf: new Uint8Array(0), seq: 0 });
    },
    data(sock, chunk) {
      const st = conns.get(sock)!;
      for (const f of drainFrames(st, new Uint8Array(chunk))) {
        switch (f.type) {
          case T.AUTH:
            sock.write(frame(T.AUTH_OK, jsonBody({ maxFrame: 1 << 20, ackWindowBytes: 256 * 1024, heartbeatSec: 15 })));
            break;
          case T.ATTACH: {
            const a = JSON.parse(new TextDecoder().decode(f.body)) as { sessionId: string; cols: number; rows: number };
            sock.write(frame(T.ATTACHED, jsonBody({ sessionId: a.sessionId, channel: 1, seq: 0, cols: a.cols, rows: a.rows, mode: "live" })));
            streamSession(sock, st, a.cols, a.rows);
            break;
          }
          case T.PING:
            sock.write(frame(T.PONG, f.body.slice(0, 8)));
            break;
          case T.RESIZE: {
            const dv = new DataView(f.body.buffer, f.body.byteOffset, f.body.length);
            sock.write(frame(T.RESIZED, u32u16u16(dv.getUint32(0, true), dv.getUint16(4, true), dv.getUint16(6, true))));
            break;
          }
          case T.INPUT:
          case T.ACK:
          case T.DETACH:
            break; // acknowledged implicitly / logged by the driver
          default:
            break;
        }
      }
    },
    close() {},
    error() {},
  },
});

console.log(`FAKE_SERVER_LISTENING port=${server.port}`);
