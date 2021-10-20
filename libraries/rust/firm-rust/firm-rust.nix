{ base, types }:
base.languages.rust.mkLibrary {
  name = "firm-rust";
  src = ./.;
  defaultTarget = "wasm32-wasi";
  targets = [ "wasm32-wasi" ];
  useNightly = "2021-05-30";
  propagatedBuildInputs = [ types.package ];
  testFeatures = [ "net" "mock" ];
  buildFeatures = [ "net" "runtime" ];
}
