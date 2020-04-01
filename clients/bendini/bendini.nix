{ pkgs, base }:
base.languages.rust.mkClient {
  name = "bendini";
  src = ./.;
}
