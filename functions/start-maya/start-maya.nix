{ pkgs, base }:
with pkgs;
base.mkComponent {
  package = stdenv.mkDerivation rec {
    name = "start-maya";
    src = builtins.filterSource
      (path: type: (type != "directory" || baseNameOf path != "target")) ./.;

    buildInputs = with pkgs; [
      cacert
      (
        latest.rustChannels.stable.rust.override {
          extensions = [ "rust-src" ];
          targets = [ "wasm32-wasi" ];
        }
      )
    ];

    manifest = ./function.toml;

    # this is needed on NixOS but does not hurt on other
    # OSes either
    PROTOC = "${pkgs.protobuf}/bin/protoc";

    buildPhase = ''
      export HOME=$PWD
      cargo build --release
    '';

    checkPhase = ''
      export HOME=$PWD

      cargo fmt -- --check

      cargo clippy
    '';

    installPhase = ''
      mkdir -p $out/bin
      cp target/wasm32-wasi/release/${name}.wasm $out/bin
      cp $manifest $out/manifest.toml
    '';

    # always want backtraces when building or in dev
    RUST_BACKTRACE = 1;
    PROTOBUF_DEFINITIONS_LOCATION = ../../protocols;
    shellHook = ''
      export PROTOBUF_DEFINITIONS_LOCATION=../../protocols
    '';
  };
}
