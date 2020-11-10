{ base, pkgs, protobuf }:

base.languages.rust.mkClient {
  name = "rust-protobuf-compiler";
  src = ./.;
  PROTOC = "${protobuf}/bin/protoc";
}
