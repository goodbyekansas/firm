{ base, pkgs, protocols }:
base.languages.rust.mkLibrary {
  name = "firm-types";
  src = ./.;
  propagatedBuildInputs = [ protocols ];
  buildInputs =
    pkgs.lib.optional pkgs.stdenv.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}
