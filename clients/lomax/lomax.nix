{ pkgs, base, protocols, protocolsTestHelpers }:
base.languages.rust.mkClient {
  name = "lomax";
  src = ./.;
  rustDependencies = [ protocols protocolsTestHelpers ];

  nativeBuildInputs = pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
