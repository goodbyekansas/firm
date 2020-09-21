{ base, pkgs, protocols }:
base.languages.rust.mkUtility {
  name = "wasi-function-utils";
  src = ./.;
  defaultTarget = "wasm32-wasi";
  targets = [ "wasm32-wasi" ];
  useNightly = "2020-05-01";
  rustDependencies = [ protocols ];
  testFeatures = [ "net" "mock" ];
  buildFeatures = [ "net" ];
}
