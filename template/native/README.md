# App-owned native code

Place platform native component sources here and register their shared-library outputs in `../nativedesktop.config.ts`.

- macOS components return `NSView`; SwiftUI can use `NSHostingView` or the `@nativedesktop/native/macos` helper source.
- Linux components return `GtkWidget*` and build against GTK 4.

`nd dev` and `nd build` run only the configured app-native command when its inputs are newer than its output. They do not rebuild NativeDesktop.
