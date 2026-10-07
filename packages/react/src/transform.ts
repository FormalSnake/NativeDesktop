// babel-preset-solid's universal transform, shared by the runtime plugin
// (register.ts) and the ahead-of-time build (build.ts). The compiled output
// imports its helpers from @nativedesktop/react.
/// <reference path="./babel-presets.d.ts" />
import { transformAsync, type PluginItem, type TransformOptions } from "@babel/core";
import presetTypescript from "@babel/preset-typescript";
import presetSolid from "babel-preset-solid";

export interface SolidTransformOptions {
  plugins?: PluginItem[];
  inputSourceMap?: TransformOptions["inputSourceMap"];
}

export async function transformSolid(source: string, filename: string, options: SolidTransformOptions = {}): Promise<string> {
  const out = await transformAsync(source, {
    filename,
    babelrc: false,
    configFile: false,
    sourceMaps: "inline",
    inputSourceMap: options.inputSourceMap,
    plugins: options.plugins ?? [],
    presets: [
      [presetTypescript, { isTSX: true, allExtensions: true }],
      [presetSolid, { generate: "universal", moduleName: "@nativedesktop/react" }],
    ],
  });
  if (!out?.code) throw new Error(`babel-preset-solid produced no output for ${filename}`);
  return out.code;
}
