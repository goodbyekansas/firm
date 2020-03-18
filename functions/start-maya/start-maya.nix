let
  moz_overlay = import (builtins.fetchTarball
    "https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz");
  nixpkgs = import <nixpkgs> { overlays = [ moz_overlay ]; };
in with nixpkgs;
stdenv.mkDerivation rec {
  name = "start-maya";
  src = builtins.filterSource
    (path: type: (type != "directory" || baseNameOf path != "target")) ./.;

  buildInputs = with nixpkgs; [
    cacert 
    (latest.rustChannels.stable.rust.override {
      extensions = ["rust-src"];
      targets = ["wasm32-wasi"];
    })
  ];

  manifest = ./function.toml;

  buildPhase = ''
    export HOME=$PWD
    cargo build --release
  '';

  installPhase = ''
    mkdir -p $out/bin
    cp target/wasm32-wasi/release/${name}.wasm $out/bin
    cp $manifest $out/manifest.toml
  '';

  # always want backtraces when building or in dev
  RUST_BACKTRACE = 1;
  PROTOBUF_DEFINITIONS_LOCATION=../../protocols;
  shellHook = ''
    export PROTOBUF_DEFINITIONS_LOCATION=../../protocols
  '';
}
