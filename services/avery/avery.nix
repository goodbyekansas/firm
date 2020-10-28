{ pkgs, base, protocols, protocolsTestHelpers }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
  rustDependencies = [ protocols protocolsTestHelpers ];
  nativeBuildInputs = pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
