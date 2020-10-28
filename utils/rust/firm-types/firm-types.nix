{ base, pkgs, protocols, protocolsTestHelpers }:
base.languages.rust.mkUtility {
  name = "firm-types";
  src = ./.;
  rustDependencies = [ protocols protocolsTestHelpers ];
}
