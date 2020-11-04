{ base, pkgs, protocols }:
base.languages.rust.mkUtility {
  name = "firm-types";
  src = ./.;
  rustDependencies = [ protocols ];
}
