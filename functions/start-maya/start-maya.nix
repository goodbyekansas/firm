{ pkgs, base, rustGbkUtils }:
base.languages.rust.mkRustFunction {
  manifest = ./function.toml;
  name = "start-maya";
  src = ./.;
  rustDependencies = [ rustGbkUtils ];
}
