{ base, types }:
base.languages.rust.mkLibrary {
  name = "firm-rust";
  src = ./.;
  defaultTarget = "wasi";
  useNightly = "2021-11-22";
  propagatedBuildInputs = [ types ];
  testFeatures = [ "net" "mock" ];
  buildFeatures = [ "net" "runtime" ];
}
