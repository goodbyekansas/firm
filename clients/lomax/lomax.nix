{ pkgs, base, avery }:
base.languages.rust.mkRustClient {
  name = "lomax";
  src = ./.;

  manifestRsPath = "${avery.package.src}/src/manifest.rs";
}
