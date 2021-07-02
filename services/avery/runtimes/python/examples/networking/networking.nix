{ base, pkgs }:
base.languages.python.mkFunction {
  name = "networking";
  version = "1.0.0";
  src = ./.;
  entrypoint = "networking:main";

  inputs = {
    port = {
      type = "int";
    };
  };
}
