{ pkgs, base, protocols, protocolsTestHelpers, tonicMiddleware }:
base.languages.rust.mkClient {
  name = "lomax";
  src = ./.;
  rustDependencies = [ protocols protocolsTestHelpers tonicMiddleware ];
  nativeBuildInputs = [ pkgs.pkg-config pkgs.openssl ]
    ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
