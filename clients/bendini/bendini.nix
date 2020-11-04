{ pkgs, base, types, tonicMiddleware }:
base.languages.rust.mkClient {
  name = "bendini";
  src = ./.;
  rustDependencies = [ types tonicMiddleware ];
  nativeBuildInputs = [ pkgs.pkg-config pkgs.openssl ]
    ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
