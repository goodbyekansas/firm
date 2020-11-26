{ pkgs, base, types }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
  externalDependenciesHash = "sha256-sh8Ea6t1BhxHu1R0XkSbYAWVT0YkJbnR8q/8Riz253k=";
  buildInputs = [ types.package ] ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;

  nativeBuildInputs = pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.xcbuild;
}
