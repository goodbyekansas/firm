{ base, pkgs, protobuf, rustProtoCompiler, includeServices }:

base.mkComponent {
  # TODO: this should really use mkUtility and not freestyle it
  # there are some issues to solve for that to work though
  package = pkgs.stdenv.mkDerivation {
    name = "rust-protobuf-definitions";
    PROTOC = "${protobuf}/bin/protoc";

    src = (builtins.path {
      path = ../.;
      name = "rust-protobuf-definitions";
      filter = (path: type: !(type == "directory" && baseNameOf path == "target"));
    });

    buildInputs = with pkgs; [
      cacert
      (
        latest.rustChannels.stable.rust.override {
          extensions = [ "rust-src" ];
        }
      )
    ];

    inherit rustProtoCompiler;

    buildPhase = ''
      $rustProtoCompiler/bin/compiler -I ./ ${if includeServices then "--build-services" else ""} -o ./gbk-protocols/src **/*.proto
      substitute rust/Cargo.toml ./gbk-protocols/Cargo.toml --subst-var-by includeTonic ${if includeServices then "'tonic = \"0.2\"'" else "''"}

      # generate a useable lib.rs
      for f in ./gbk-protocols/src/**.rs; do
        echo "pub mod $(basename "$f" .rs);" >> ./gbk-protocols/src/lib.rs
      done
      ${if includeServices then "echo 'pub use tonic;' >> ./gbk-protocols/src/lib.rs" else "" }
    '';

    checkPhase = ''
      # check that it works
      export CARGO_HOME=$PWD
      cargo test --manifest-path=./gbk-protocols/Cargo.toml
      rm -rf ./gbk-protocols/target ./gbk-protocols/Cargo.lock
    '';

    installPhase = ''
      mkdir -p $out
      cp -r gbk-protocols/* $out/
    '';

  };
}
