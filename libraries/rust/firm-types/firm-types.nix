{ base, pkgs, protocols }:
base.languages.rust.mkLibrary {
  name = "firm-types";
  src = ./.;
  propagatedBuildInputs = [ protocols.package ];
  buildInputs =
    pkgs.lib.optional pkgs.stdenv.isDarwin pkgs.darwin.apple_sdk.frameworks.Security;
}