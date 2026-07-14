import { defineConfig } from "nd/config";

export default defineConfig({
  // Add app-owned native libraries here. See docs/native-components.md.
  // Build commands run with ND_NATIVE_PACKAGE set to the @nativedesktop/native
  // package root — use it for include paths and the Swift helper source.
  native: { plugins: [] },

  // File associations + URL schemes flow into macOS Info.plist
  // (CFBundleDocumentTypes/CFBundleURLTypes) and the Linux .desktop's
  // MimeType= at `nd package` time. See docs/packaging.md.
  // app: {
  //   id: "com.example.myapp",
  //   fileAssociations: [{ ext: "md", name: "Markdown Document", mimeType: "text/markdown" }],
  //   urlSchemes: [{ scheme: "myapp" }],
  // },
});
