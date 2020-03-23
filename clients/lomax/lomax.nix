{ pkgs, base }:
base.languages.rust.mkRustClient {
  name = "lomax";
  src = ./.;
}
