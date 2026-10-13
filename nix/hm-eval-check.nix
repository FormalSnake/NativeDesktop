# Evaluates an app's home-manager module against stub home-manager options, without home-manager
# itself. Evaluation errors surface during `nix flake check --no-build`; the derivation then checks
# the rendered file.
{ lib }:
{
  pkgs,
  pname,
  homeModule,
  # Settings that must evaluate and render.
  settings,
  # Lines (whole-line, fixed strings) that config.toml must contain.
  expectLines,
  # Strings that must not appear in config.toml.
  absent ? [ ],
  # Settings that must be rejected at evaluation: wrong type, out of range, unknown key.
  rejects ? [ ],
}:
let
  stub =
    { lib, ... }:
    {
      options = {
        assertions = lib.mkOption {
          type = lib.types.listOf lib.types.unspecified;
          default = [ ];
        };
        home.packages = lib.mkOption {
          type = lib.types.listOf lib.types.package;
          default = [ ];
        };
        home.file = lib.mkOption {
          type = lib.types.attrsOf lib.types.attrs;
          default = { };
        };
        xdg.configFile = lib.mkOption {
          type = lib.types.attrsOf lib.types.attrs;
          default = { };
        };
        systemd.user.services = lib.mkOption {
          type = lib.types.attrsOf lib.types.attrs;
          default = { };
        };
        launchd.agents = lib.mkOption {
          type = lib.types.attrsOf lib.types.attrs;
          default = { };
        };
      };
    };

  evaluate =
    s:
    (lib.evalModules {
      specialArgs = { inherit pkgs; };
      modules = [
        stub
        homeModule
        {
          programs.${pname} = {
            enable = true;
            package = pkgs.hello;
            settings = s;
          };
        }
      ];
    }).config;

  config = evaluate settings;
  isDarwin = pkgs.stdenv.hostPlatform.isDarwin;
  rendered =
    if isDarwin then
      config.home.file."Library/Application Support/${pname}/config.toml".source
    else
      config.xdg.configFile."${pname}/config.toml".source;

  rejected = lib.all (s: !(builtins.tryEval (builtins.deepSeq (evaluate s).programs.${pname}.settings true)).success) rejects;
  failedAssertions = lib.filter (a: !a.assertion) config.assertions;
in
assert lib.assertMsg rejected "hm-module-eval ${pname}: a settings value that should be rejected evaluated";
assert lib.assertMsg (failedAssertions == [ ]) "hm-module-eval ${pname}: ${
  lib.concatMapStringsSep "; " (a: a.message) failedAssertions
}";
pkgs.runCommand "hm-module-eval-${pname}" { } ''
  ${lib.concatMapStrings (l: "grep -qxF ${lib.escapeShellArg l} ${rendered}\n") expectLines}
  ${lib.concatMapStrings (a: "! grep -qF ${lib.escapeShellArg a} ${rendered}\n") absent}
  touch $out
''
