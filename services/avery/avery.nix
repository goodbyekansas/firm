{ pkgs, base, types, tonicMiddleware }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
  buildInputs = [ types.package tonicMiddleware.package ] ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
  nativeBuildInputs = pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.xcbuild;
}
