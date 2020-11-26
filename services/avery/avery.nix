{ pkgs, base, types, tonicMiddleware }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
  buildInputs = [ types.package tonicMiddleware.package ] ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
  nativeBuildInputs = pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.xcbuild;

  externalDependenciesHash = "sha256-QcYvBojWLbh3uVAbeBCc63SV9Yi0oNrRv0a+e4u+yLs=";
}
