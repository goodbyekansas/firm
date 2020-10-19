{ pkgs, base, protocols, protocolsTestHelpers, tonicMiddleware }:
base.languages.rust.mkClient {
  name = "bendini";
  src = ./.;
  RUSTFLAGS = "-D warnings"; # TODO: This should be remove once nedryland has been updated with default
  rustDependencies = [ protocols protocolsTestHelpers tonicMiddleware ];
  nativeBuildInputs = [ pkgs.pkg-config pkgs.openssl ]
    ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
