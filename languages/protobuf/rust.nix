{ name, protoSources, version, rust, includeServices, protobuf, stdenv, rustfmt, protoIncludePaths }:
let
  tonicDependencyString = ''tonic = { version = "0.3", features = ["tls", "tls-roots"] }'';
in
stdenv.mkDerivation {
  inherit protoSources protoIncludePaths;
  rustProtoCompiler = (import ./rust/compiler { inherit rust protobuf; }).package;
  name = "rust-${name}";

  PROTOC = "${protobuf}/bin/protoc";

  src = builtins.path { path = ./rust/src; inherit name; };

  # seem to need rustfmt, prob run on the resulting code
  nativeBuildInputs = [ rustfmt ];

  buildPhase = ''
    shopt -s extglob globstar nullglob
    includes=""
    for p in $protoIncludePaths; do
      includes+=" -I $p"
    done

    $rustProtoCompiler/bin/rust-protobuf-compiler \
      -I $protoSources \
      $includes \
      ${if includeServices then "--build-services" else ""} \
      -o ./src \
      $protoSources/**/*.proto

    substituteInPlace ./Cargo.toml \
      --subst-var-by includeTonic ${if includeServices then "'${tonicDependencyString}'" else "''"} \
      --subst-var-by packageName ${name} \
      --subst-var-by version ${version}

    # generate a useable lib.rs
    echo "// Generated by firm, do not edit" > ./src/lib.rs

    for f in ./src/**/!(lib).rs; do
      echo "pub mod $(basename "$f" .rs);" >> ./src/lib.rs
    done

    ${if includeServices then "echo 'pub use tonic;' >> ./src/lib.rs" else "" }
  '';

  installPhase = ''
    mkdir $out
    cp -r Cargo.toml src $out/
  '';
}
