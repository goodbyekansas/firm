{ pkgs, base, protocols, protocolsTestHelpers }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
  rustDependencies = [ protocols protocolsTestHelpers ];
  RUSTFLAGS = "-D warnings"; # TODO: This should be remove once nedryland has been updated with default
  nativeBuildInputs = pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
