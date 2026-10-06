// The Solid JSX transform is a runtime Bun plugin, so it has to be registered
// before the first .tsx module loads: hence a .ts entry and a dynamic import.
import "@nativedesktop/solid/register";

await import("./app.tsx");
