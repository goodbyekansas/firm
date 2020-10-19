{ base, pkgs, protocols }:
base.languages.rust.mkUtility {
  name = "rust-protobuf-test-helpers";
  src = ./.;
  rustDependencies = [ protocols ];
}
