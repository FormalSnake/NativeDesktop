// Compiled JSX imports its helpers from here (babel-preset-solid's universal
// moduleName), so these names are the renderer's contract with the compiler.
export {
  effect,
  memo,
  createComponent,
  createElement,
  createTextNode,
  insertNode,
  insert,
  spread,
  setProp,
  mergeProps,
  applyRef,
  ref,
} from "./renderer.ts";
export { render, Portal, createPool, nextCommit } from "./renderer.ts";
export type { Pool, SolidNode } from "./renderer.ts";
export { defineNativeComponent } from "./native-component.ts";
export type { NativeComponentOptions, NativeComponentProps, NativeComponentRef } from "./native-component.ts";
export { useStoreValue } from "./store.ts";
export type { JSX } from "./generated/intrinsics.ts";
export { sendCommand, sendNativeCommand, moveNode } from "@nativedesktop/core";
export { Platform, hasWidget, hasCommand } from "@nativedesktop/core";
export type { Backend, OS } from "@nativedesktop/core";
export { Spacing, ContentMargin, ContentWidth } from "@nativedesktop/core";
export type { SpacingScale } from "@nativedesktop/core";
export { getAppDataDir, ensureAppDataDir } from "@nativedesktop/core";
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
} from "@nativedesktop/core";
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
} from "@nativedesktop/core";
export {
  showAlert,
  openFile,
  saveFile,
  showAbout,
  showTabOverview,
  onAlertResult,
  onOpenFileResult,
  onSaveFileResult,
} from "@nativedesktop/core";
export type {
  DialogButton,
  ShowAlertOptions,
  AlertResult,
  WindowOpenFileOptions,
  WindowOpenFileResult,
  WindowSaveFileOptions,
  WindowSaveFileResult,
  ShowAboutOptions,
} from "@nativedesktop/core";
export { showToast, dismissToast, onToastButtonClicked, onToastDismissed } from "@nativedesktop/core";
export type { ToastPriority, ShowToastOptions, ToastResult } from "@nativedesktop/core";
export type {
  NdNodeRef,
  WidgetType,
  TableColumn,
  TableRow,
  TreeNode,
  SourceTreeAction,
  SourceTreeNode,
  CommandPaletteItem,
  ChartPoint,
  ChartSeries,
  CodeDiagnostic,
  MenuEntry,
} from "@nativedesktop/core";
export { dialog, clipboard, notifications, recentDocuments, credentials, app, system, audio, webviewEngine } from "@nativedesktop/core";
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
} from "@nativedesktop/core";
export { openExternal, openPath, revealPath } from "@nativedesktop/core";
export { onUnhandledError, setUnhandledErrorPolicy } from "@nativedesktop/core";
export type { NdErrorKind, NdErrorContext, NdErrorHandler, UnhandledErrorPolicy } from "@nativedesktop/core";
export { createStore } from "@nativedesktop/core";
export type { Store, StoreOptions } from "@nativedesktop/core";
