{ base, pkgs, protocols }:
base.languages.rust.mkUtility {
  name = "tonic-middleware";
  src = ./.;
  propagatedBuildInputs = [ protocols.package ];
}
