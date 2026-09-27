import { render } from "@nativedesktop/react";

// One chromium view, so the profile and the fixture extension's worker come
// up. scripts/native-messaging-drive.ts does the rest over CDP.
await render(
  <window testID="nm-window" title="ND native messaging" defaultWidth={640} defaultHeight={400}>
    <webview testID="nm-view" url="about:blank" style={{ hexpand: true, vexpand: true }} />
  </window>,
);
