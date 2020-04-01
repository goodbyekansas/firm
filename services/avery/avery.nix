{ pkgs, base }:
with pkgs;
base.languages.rust.mkService {
  name = "avery";
  src = ./.;
}
