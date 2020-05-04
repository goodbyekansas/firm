{ pkgs, base, protocols }:
base.languages.rust.mkClient {
  name = "lomax";
  src = ./.;
  rustDependencies = [ protocols ];
}
