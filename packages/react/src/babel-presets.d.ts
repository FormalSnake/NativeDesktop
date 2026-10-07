// Neither preset ships types; both export a plain babel preset function.
declare module "@babel/preset-typescript" {
  const preset: import("@babel/core").PluginItem;
  export default preset;
}
declare module "babel-preset-solid" {
  const preset: import("@babel/core").PluginItem;
  export default preset;
}
