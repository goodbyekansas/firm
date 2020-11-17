{ base, pkgs }:
base.languages.protobuf.mkModule {
  name = "firm-protocols";
  src = ./.;
  version = "0.1.0";
  languages = [ "rust" "python" ];
  pythonVersion = pkgs.python3;
}
