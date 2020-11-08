{ pkgs, base, types }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
  externalDependenciesHash = "sha256-P8sPV/IuQHp5Jh89e1FtUZUi0+sxj1xsLP0pNJ3S3GU=";
  nativeBuildInputs = pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
  buildInputs = [ types.package ];
}
