// App-facing APIs (re-exported by every renderer package).
export { sendCommand, sendNativeCommand, moveNode } from "./commands.ts";
export { Platform, hasWidget, hasCommand } from "./platform.ts";
export type { Backend, OS } from "./platform.ts";
export { Spacing, ContentMargin, ContentWidth } from "./metrics.ts";
export type { SpacingScale } from "./metrics.ts";
export { getAppDataDir, ensureAppDataDir } from "./paths.ts";
export {
  executeJavaScript,
  onJavaScriptResult,
  getCookies,
  onCookiesResult,
  saveSession,
  onSessionSaved,
  setContextMenuItems,
  listExtensions,
  onExtensionsList,
  listExtensionActions,
  readExtensionAction,
  triggerExtensionAction,
  onExtensionActions,
  watchExtensions,
  onExtensionsChanged,
  installExtension,
  uninstallExtension,
  setExtensionEnabled,
  respondDownload,
  startDownload,
  pauseDownload,
  resumeDownload,
  cancelDownload,
  newWindowRequest,
  acceptExtensionInstall,
} from "./webview.ts";
export type {
  Cookie,
  DownloadRequest,
  DownloadState,
  DownloadUpdate,
  ContextMenuContext,
  ContextMenuItem,
  ContextMenuItemClick,
  ExtensionAction,
  ExtensionActionState,
  ExtensionsChange,
  InstalledExtension,
  NewWindowDisposition,
  NewWindowRequest,
} from "./webview.ts";
export {
  showAlert,
  openFile,
  saveFile,
  showAbout,
  showTabOverview,
  onAlertResult,
  onOpenFileResult,
  onSaveFileResult,
} from "./dialogs.ts";
// Aliased to Window* on export: system.ts's ACL-gated `dialog.openFile`/
// `dialog.saveFile` (app-level, not window-scoped) already own the
// unprefixed OpenFileOptions/SaveFileOptions names below.
export type {
  DialogButton,
  ShowAlertOptions,
  AlertResult,
  OpenFileOptions as WindowOpenFileOptions,
  OpenFileResult as WindowOpenFileResult,
  SaveFileOptions as WindowSaveFileOptions,
  SaveFileResult as WindowSaveFileResult,
  ShowAboutOptions,
} from "./dialogs.ts";
export { showToast, dismissToast, onToastButtonClicked, onToastDismissed } from "./toast.ts";
export type { ToastPriority, ShowToastOptions, ToastResult } from "./toast.ts";
export { dialog, clipboard, notifications, recentDocuments, credentials, app, system, audio, webviewEngine } from "./system.ts";
export type {
  FileFilter,
  OpenFileOptions,
  SaveFileOptions,
  MessageLevel,
  MessageOptions,
  NotificationOptions,
  Appearance,
  AppearanceInfo,
  AudioPlayOptions,
  AudioState,
  AudioStateEvent,
  AudioSpectrumEvent,
  ContentBlockingList,
  ContentBlockingLoadOptions,
  ContentBlockingConfigureOptions,
} from "./system.ts";
export { openExternal, openPath, revealPath } from "./shell.ts";
export { onUnhandledError, setUnhandledErrorPolicy } from "./errors.ts";
export type { NdErrorKind, NdErrorContext, NdErrorHandler, UnhandledErrorPolicy } from "./errors.ts";
export { createStore } from "./store.ts";
export type { Store, StoreOptions } from "./store.ts";
export type * from "./generated/widgets.ts";
export type { Op, CommitBatch, EventMsg } from "../../../runtime/ndp.ts";

// Renderer SDK: what a renderer package builds its host bindings from.
export { connect, getSession, setSession, isHot } from "./session.ts";
export type { Session, ConnectOptions } from "./session.ts";
export { Batch, NodeRegistry, onNodeRemoved } from "./ops.ts";
export type { Handler, NodeRecord } from "./ops.ts";
export { currentGeneration, newGeneration, nextNodeId } from "./ids.ts";
export {
  collectHandlers,
  eventForHandler,
  warnUnknownHandler,
  refTargetId,
  propsEqual,
  removalValue,
  checkPlatform,
} from "./props.ts";
export { validateStyle, StyleError } from "./style-validate.ts";
export { validateCssClasses, CssClassError } from "./css-classes-validate.ts";
export { installErrorHandlers, reportRenderError } from "./errors.ts";
export { styleKeySpec, cssClassSpec } from "./generated/widgets.ts";
export {
  intrinsicToName,
  widgetEvents,
  handlerPropNames,
  widgetPlatforms,
  widgetRefProps,
  widgetCommands,
} from "./generated/schema-meta.ts";
export type { WidgetCommandNames } from "./generated/schema-meta.ts";
