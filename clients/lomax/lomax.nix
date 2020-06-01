{ pkgs, base, protocols, protocolsTestHelpers }:
base.languages.rust.mkClient {
  name = "lomax";
  src = ./.;
  rustDependencies = [ protocols protocolsTestHelpers];
}
