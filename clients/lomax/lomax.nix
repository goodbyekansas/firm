{ pkgs, base }:
base.languages.rust.mkClient {
  name = "lomax";
  src = ./.;
}
