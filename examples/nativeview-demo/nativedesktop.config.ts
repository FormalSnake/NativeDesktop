import { defineConfig } from "@nativedesktop/cli/config";

// nd dev/build run these with ND_NATIVE_PACKAGE set to the installed
// @nativedesktop/native root and, on darwin, a cleaned Xcode toolchain env.
export default defineConfig({
  native: {
    plugins: [{
      darwin: "native/build/libcolorview.dylib",
      linux: "native/build/libcolorview.so",
      build: {
        darwin: {
          command: ["bash", "-c", "mkdir -p native/build && /usr/bin/xcrun --sdk macosx swiftc -emit-library -o native/build/libcolorview.dylib -I \"$ND_NATIVE_PACKAGE/macos\" native/macos/ColorView.swift \"$ND_NATIVE_PACKAGE/macos/NativeDesktopNative.swift\" -framework AppKit -framework SwiftUI"],
          inputs: ["native/macos", "node_modules/@nativedesktop/native"],
        },
        linux: {
          command: ["bash", "-c", "mkdir -p native/build && cc -shared -fPIC $(pkg-config --cflags gtk4) -I \"$ND_NATIVE_PACKAGE/include\" -I \"$ND_NATIVE_PACKAGE/linux\" native/linux/colorview.c -o native/build/libcolorview.so $(pkg-config --libs gtk4)"],
          inputs: ["native/linux", "node_modules/@nativedesktop/native"],
        },
      },
    }],
  },
});
