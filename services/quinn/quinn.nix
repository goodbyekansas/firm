{ pkgs, base, protocols, protocolsTestHelpers }:
with pkgs;
base.languages.rust.mkService {
  name = "quinn";
  src = ./.;
  rustDependencies = [ protocols protocolsTestHelpers ];
}
