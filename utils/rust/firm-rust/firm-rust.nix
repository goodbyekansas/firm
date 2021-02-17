{ base, pkgs, types }:
base.languages.rust.mkUtility {
  name = "firm-rust";
  src = ./.;
  defaultTarget = "wasm32-wasi";
  targets = [ "wasm32-wasi" ];
  useNightly = "2021-01-27";
  propagatedBuildInputs = [ types.package ];
  testFeatures = [ "net" "mock" ];
  buildFeatures = [ "net" "runtime" ];
}