{ pkgs, base, types, tonicMiddleware }:
base.languages.rust.mkClient {
  name = "bendini";
  src = ./.;
  externalDependenciesHash = "sha256-aTupbDKk5Vdjx4L2NS1j9jHZ+CK8kcuEXFvho0pjQ9s=";
  buildInputs = [ types.package tonicMiddleware.package ];
  nativeBuildInputs = [ pkgs.pkg-config pkgs.openssl ]
    ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
