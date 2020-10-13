{ base, pkgs, protocols }:
base.languages.rust.mkUtility {
  name = "tonic-middleware";
  src = ./.;
  rustDependencies = [ protocols ];
}
