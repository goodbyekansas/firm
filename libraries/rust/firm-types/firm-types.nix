{ base, lib, stdenv, darwin, protocols }:
base.languages.rust.mkLibrary {
  name = "firm-types";
  src = ./.;
  propagatedBuildInputs = [ protocols ];
  buildInputs =
    lib.optional stdenv.isDarwin darwin.apple_sdk.frameworks.Security;
}
