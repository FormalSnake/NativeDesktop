import {
  render,
  useRef,
  useState,
  dialog,
  showAlert,
  openFile,
  saveFile,
  onAlertResult,
  onOpenFileResult,
  onSaveFileResult,
} from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// Minimal, untabbed native-dialog surface for scripts/dialog-script-drive.ts.
// examples/gallery wires the same six calls, but behind a TabView page
// ("Dialogs") — there is no automation RPC to switch tabs (a documented gap
// in automation-socket.md), so every trigger there is unreachable by ref/
// testID until a user (or real input synthesis) clicks the tab strip. This
// app puts all six triggers directly on the window so ND_AUTOMATION_DIALOG_SCRIPT
// (§1.5) can be exercised on both backends without that dependency.
//
// Covers both interception paths: the app-level `dialog.*` systemRequest
// (packages/react/src/system.ts, ACL-gated but default-granted) and the
// window-scoped showAlert/openFile/saveFile widgetCommand (packages/react/
// src/dialogs.ts).

function App(): React.ReactNode {
  const winRef = useRef<NdNodeRef<"window">>(null);

  const [appOpenFileResult, setAppOpenFileResult] = useState("(none yet)");
  const [appSaveFileResult, setAppSaveFileResult] = useState("(none yet)");
  const [appShowMessageResult, setAppShowMessageResult] = useState("(none yet)");
  const [windowAlertResult, setWindowAlertResult] = useState("(none yet)");
  const [windowOpenFileResult, setWindowOpenFileResult] = useState("(none yet)");
  const [windowSaveFileResult, setWindowSaveFileResult] = useState("(none yet)");

  async function handleAppOpenFile(): Promise<void> {
    const paths = await dialog.openFile();
    setAppOpenFileResult(paths.length ? paths.join(", ") : "(none)");
  }

  async function handleAppSaveFile(): Promise<void> {
    const path = await dialog.saveFile({ defaultName: "export.json" });
    setAppSaveFileResult(path ?? "(canceled)");
  }

  async function handleAppShowMessage(): Promise<void> {
    const index = await dialog.showMessage({ message: "Discard changes?", buttons: ["Cancel", "Discard"] });
    setAppShowMessageResult(String(index));
  }

  async function handleWindowShowAlert(): Promise<void> {
    if (!winRef.current) return;
    const result = await showAlert(winRef.current, {
      title: "Delete this item?",
      body: "This action cannot be undone.",
      buttons: [
        { id: "cancel", label: "Cancel" },
        { id: "delete", label: "Delete", style: "destructive" },
      ],
    });
    setWindowAlertResult(result.buttonId);
  }

  async function handleWindowOpenFile(): Promise<void> {
    if (!winRef.current) return;
    const result = await openFile(winRef.current, { filters: [{ name: "Text", extensions: ["txt", "md"] }] });
    setWindowOpenFileResult(result.canceled ? "canceled" : result.paths.join(", "));
  }

  async function handleWindowSaveFile(): Promise<void> {
    if (!winRef.current) return;
    const result = await saveFile(winRef.current, { suggestedName: "export.json" });
    setWindowSaveFileResult(result.canceled ? "canceled" : (result.path ?? "(null)"));
  }

  return (
    <window
      ref={winRef}
      title="NativeDesktop Dialogs"
      defaultWidth={520}
      defaultHeight={420}
      onAlertResult={(e) => onAlertResult(winRef.current!, e)}
      onOpenFileResult={(e) => onOpenFileResult(winRef.current!, e)}
      onSaveFileResult={(e) => onSaveFileResult(winRef.current!, e)}
    >
      <box orientation="vertical" spacing={10} style={{ padding: 12 }}>
        <box orientation="horizontal" spacing={8}>
          <button testID="app-open-file-button" label="App: Open File…" onClick={handleAppOpenFile} />
          <label testID="app-open-file-result" text={`Result: ${appOpenFileResult}`} cssClasses={["dimmed", "caption"]} />
        </box>
        <box orientation="horizontal" spacing={8}>
          <button testID="app-save-file-button" label="App: Save File…" onClick={handleAppSaveFile} />
          <label testID="app-save-file-result" text={`Result: ${appSaveFileResult}`} cssClasses={["dimmed", "caption"]} />
        </box>
        <box orientation="horizontal" spacing={8}>
          <button testID="app-show-message-button" label="App: Show Message" onClick={handleAppShowMessage} />
          <label testID="app-show-message-result" text={`Result: ${appShowMessageResult}`} cssClasses={["dimmed", "caption"]} />
        </box>
        <box orientation="horizontal" spacing={8}>
          <button testID="window-show-alert-button" label="Window: Show Alert" onClick={handleWindowShowAlert} />
          <label testID="window-alert-result" text={`Result: ${windowAlertResult}`} cssClasses={["dimmed", "caption"]} />
        </box>
        <box orientation="horizontal" spacing={8}>
          <button testID="window-open-file-button" label="Window: Open File…" onClick={handleWindowOpenFile} />
          <label testID="window-open-file-result" text={`Result: ${windowOpenFileResult}`} cssClasses={["dimmed", "caption"]} />
        </box>
        <box orientation="horizontal" spacing={8}>
          <button testID="window-save-file-button" label="Window: Save File…" onClick={handleWindowSaveFile} />
          <label testID="window-save-file-result" text={`Result: ${windowSaveFileResult}`} cssClasses={["dimmed", "caption"]} />
        </box>
      </box>
    </window>
  );
}

await render(<App />);
