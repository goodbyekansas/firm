{ pkgs, base, types, protocolsTestHelpers, tonicMiddleware }:
base.languages.rust.mkClient {
  name = "bendini";
  src = ./.;
  rustDependencies = [ types protocolsTestHelpers tonicMiddleware ];
  nativeBuildInputs = [ pkgs.pkg-config pkgs.openssl ]
    ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
