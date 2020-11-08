{ base, pkgs, types }:
base.languages.rust.mkUtility {
  name = "firm-rust";
  src = ./.;
  defaultTarget = "wasm32-wasi";
  targets = [ "wasm32-wasi" ];
  useNightly = "2020-10-25";
  propagatedBuildInputs = [ types.package ];
  testFeatures = [ "net" "mock" ];
  buildFeatures = [ "net" ];
}
