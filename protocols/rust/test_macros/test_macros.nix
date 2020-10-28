{ base, pkgs, protocols }:
base.languages.rust.mkUtility {
  name = "rust-firm-protocols-test-helpers";
  src = ./.;
  rustDependencies = [ protocols ];
}
