{ base, pkgs, protocols }:
base.languages.rust.mkUtility {
  name = "gbk-protocols-test-helpers";
  src = ./.;
  rustDependencies = [ protocols ];
}
