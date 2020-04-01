{ pkgs, base, rustGbkUtils }:
base.languages.rust.mkFunction {
  manifest = ./function.toml;
  name = "start-maya";
  src = ./.;
  rustDependencies = [ rustGbkUtils ];
}
