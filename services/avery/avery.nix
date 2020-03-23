{ pkgs, base, inputFunctions }:
with pkgs;
base.languages.rust.mkRustService {
  name = "avery";
  src = ./.;

  # TODO: This gives a nice workflow for now but should be removed later
  inputFunctions = builtins.map (c: c.package) inputFunctions;
}
