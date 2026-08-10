import { defineConfig } from "nd/config";

export default defineConfig({
  app: {
    id: "com.nativedesktop.gallery",
    name: "Gallery",
    displayName: "NativeDesktop Gallery",
    version: "0.9.0",
  },
  package: {
    entry: "main.tsx",
    compile: false,
    // The bundle app root mirrors the repo root, so the gallery lands at
    // app/examples/gallery and the gates' counter launch path keeps working.
    workspaceRoot: "../..",
    include: ["examples/counter", "runtime"],
    outDir: "dist",
    updates: { baseUrl: "http://127.0.0.1:0", ephemeralKey: true },
  },
});
