import { render } from "@nativedesktop/react";

// Negative test: an invalid web-CSS style key must be rejected at mount
// with the fix-it StyleError (see packages/react/src/style-validate.ts).
// Built as Record<string, unknown> (not a StyleProp literal) so the TS
// excess-property check doesn't shadow the runtime rejection this script
// exists to exercise. Run standalone via
// ND_SCRIPT=examples/gallery/gallery-badstyle.tsx — this process is expected
// to throw and exit nonzero (scripts/headless-m5c.sh greps stderr for the
// fix-it message).
const badStyle: Record<string, unknown> = { display: "flex" };

function App(): React.ReactNode {
  return <label style={badStyle} />;
}

await render(<App />);
