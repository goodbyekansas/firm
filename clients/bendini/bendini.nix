let
  moz_overlay = import (builtins.fetchTarball
    "https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz");
  nixpkgs = import <nixpkgs> { overlays = [ moz_overlay ]; };
in with nixpkgs;
stdenv.mkDerivation {
  name = "bendini";
  src = builtins.filterSource
    (path: type: (type != "directory" || baseNameOf path != "target")) ./.;

  buildInputs = with nixpkgs; [
    cacert 
    (latest.rustChannels.stable.rust.override {extensions = ["rust-src"];})
  ];
  
  # this is needed on NixOS but does not hurt on other
  # OSes either
  PROTOC = "${pkgs.protobuf}/bin/protoc";

  doCheck = true; # TODO: When nedryland takes over the world remove this.

  buildPhase = ''
    export HOME=$PWD
    cargo build --release
  '';

  checkPhase = ''
    if [ -z $IN_NIX_SHELL ]; then
      export HOME="$PWD"
    fi

    cargo test

    cargo fmt -- --check

    cargo clippy
  '';

  installPhase = ''
    mkdir -p $out/bin
    cp target/release/bendini $out/bin
  '';

  # always want backtraces when building or in dev
  RUST_BACKTRACE = 1;
  PROTOBUF_DEFINITIONS_LOCATION=../../protocols;
  shellHook = ''
    export PROTOBUF_DEFINITIONS_LOCATION=../../protocols
  '';
}

