// Reads a host log from examples/adblock-probe and checks every phase against
// what uBlock Origin would do with the probe's list. Shared by the macOS and
// Linux content-blocking gates. Prints ND_ADBLOCK_OK or exits 1.
import { readFileSync } from "node:fs";

const log = readFileSync(process.argv[2]!, "utf8");

type Report = Record<string, unknown>;
const phases = new Map<string, { blocked: number; report: Report }>();
for (const m of log.matchAll(/ND_ADBLOCK_PHASE (\w+) blocked=(\d+) (\{.*\})/g)) {
  phases.set(m[1]!, { blocked: Number(m[2]), report: JSON.parse(m[3]!) as Report });
}

const blocking: Report = {
  adScript: false,
  redirectReal: false,
  redirectNoop: true,
  scriptletEarly: true,
  pixel: false,
  banner: false,
  siteAd: false,
  proc: false,
  user: true,
  child: "none",
  childScriptlet: false,
  worker: true,
};
const expected: Record<string, { report: Report; minBlocked: number; maxBlocked: number }> = {
  on: { report: blocking, minBlocked: 3, maxBlocked: 6 },
  siteOff: {
    report: {
      adScript: true,
      redirectReal: true,
      redirectNoop: false,
      scriptletEarly: false,
      pixel: true,
      banner: true,
      siteAd: true,
      proc: true,
      user: true,
      child: "block",
      worker: true,
    },
    minBlocked: 0,
    maxBlocked: 0,
  },
  userRule: { report: { ...blocking, user: false }, minBlocked: 3, maxBlocked: 6 },
};

let failed = false;
for (const [name, want] of Object.entries(expected)) {
  const got = phases.get(name);
  if (!got) {
    console.log(`FAIL ${name}: no ND_ADBLOCK_PHASE line`);
    failed = true;
    continue;
  }
  for (const [key, value] of Object.entries(want.report)) {
    if (got.report[key] !== value) {
      console.log(`FAIL ${name}.${key}: want ${JSON.stringify(value)}, got ${JSON.stringify(got.report[key])}`);
      failed = true;
    }
  }
  if (got.blocked < want.minBlocked || got.blocked > want.maxBlocked) {
    console.log(`FAIL ${name}.blocked: want ${want.minBlocked}..${want.maxBlocked}, got ${got.blocked}`);
    failed = true;
  }
  if (!failed) console.log(`ok ${name} blocked=${got.blocked}`);
}
if (failed) process.exit(1);
console.log("ND_ADBLOCK_OK");
