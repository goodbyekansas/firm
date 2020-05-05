{ base, pkgs }:

base.languages.rust.mkClient {
  name = "compiler";
  src = ./.;
}
