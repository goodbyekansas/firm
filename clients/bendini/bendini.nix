{ pkgs, base, types, tonicMiddleware }:
base.languages.rust.mkClient {
  name = "bendini";
  src = ./.;
  buildInputs = [ types.package tonicMiddleware.package ]
    ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
  nativeBuildInputs = [ pkgs.pkg-config pkgs.openssl ];

  externalDependenciesHash = "sha256-2DODt6t884Ju+wyfD4koW5mGiD/dDlL3EZTr2seCnSQ=";
}
