import { defineConfig } from "nd/config";

export default defineConfig({
  native: {
    plugins: [{
      darwin: "native/build/libcolorview.dylib",
      linux: "native/build/libcolorview.so",
      build: {
        darwin: {
          command: ["bash", "-c", "mkdir -p native/build && unset SDKROOT DEVELOPER_SDK_DIR NIX_CFLAGS_COMPILE NIX_LDFLAGS; export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer; SDK=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) && /usr/bin/xcrun --sdk macosx swiftc -sdk \"$SDK\" -emit-library -o native/build/libcolorview.dylib -I native/macos native/macos/ColorView.swift ../../packages/native/macos/NativeDesktopNative.swift -framework AppKit -framework SwiftUI"],
          inputs: ["native/macos", "../../packages/native/include"],
        },
        linux: {
          command: ["bash", "-c", "mkdir -p native/build && cc -shared -fPIC $(pkg-config --cflags gtk4) -I ../../packages/native/include native/linux/colorview.c -o native/build/libcolorview.so $(pkg-config --libs gtk4)"],
          inputs: ["native/linux", "../../packages/native/include"],
        },
      },
    }],
  },
});
