{ pkgs, base, avery }:
base.languages.rust.mkClient {
  name = "lomax";
  src = ./.;

  manifestRsPath = "${avery.package.src}/src/manifest.rs";
}
