{ pkgs, base, types, tonicMiddleware }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
  srcExclude = [
    (path: type: (type == "directory" && baseNameOf path == "runtimes"))
    (path: type: (type == "regular" && baseNameOf path == "avery-with-runtimes.nix"))
  ];
  buildInputs = [ types.package tonicMiddleware.package ]
    ++ pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
  nativeBuildInputs = pkgs.stdenv.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.xcbuild;
}
