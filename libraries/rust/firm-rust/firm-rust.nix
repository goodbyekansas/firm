{ base, types }:
(base.languages.rust.nightly.withWasi.overrideCrossTargets (targets: {
  rust = targets.wasi;
})).mkLibrary {
  name = "firm-rust";
  src = ./.;
  propagatedBuildInputs = [ types ];
  testFeatures = [ "net" "mock" ];
  buildFeatures = [ "net" "runtime" ];
}
