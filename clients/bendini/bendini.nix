{ pkgs, base }:
base.languages.rust.mkRustClient {
  name = "bendini";
  src = ./.;
}
