{ pkgs, base, protocolsTestHelpers, types }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
  rustDependencies = [ protocolsTestHelpers types ];
  nativeBuildInputs = pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
