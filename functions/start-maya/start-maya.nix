{ pkgs, base }:
base.languages.rust.mkRustFunction {
  manifest = ./function.toml;
  name = "start-maya";
  src = ./.;
}
