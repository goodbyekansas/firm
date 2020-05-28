{ pkgs, base, protocols, protocolsTestHelpers }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
  rustDependencies = [ protocols protocolsTestHelpers ];
}
