{ pkgs, base, protocols }:
base.languages.rust.mkClient {
  name = "bendini";
  src = ./.;
  rustDependencies = [ protocols ];
}
