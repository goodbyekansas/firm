{ pkgs, base, protocols }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
  rustDependencies = [ protocols ];
}
