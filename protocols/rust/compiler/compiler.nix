{ base, pkgs }:

base.languages.rust.mkClient {
  name = "rust-protobuf-compiler";
  src = ./.;
}
