{ base, pkgs }:
rec{
  source = ./.;
  withoutServices = base.languages.protobuf.mkModule {
    name = "firm-protocols";
    src = source;
    version = "0.1.0";
    languages = [ "rust" "python" ];
    pythonVersion = pkgs.python3;
    includeServices = false;
  };
  withServices = base.languages.protobuf.mkModule {
    name = "firm-protocols";
    src = source;
    version = "0.1.0";
    languages = [ "rust" "python" ];
    pythonVersion = pkgs.python3;
    includeServices = true;
  };
}
