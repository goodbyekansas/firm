{ base, pkgs }:
base.languages.python.mkFunction {
  name = "yamler";
  version = "0.1.0";
  src = ./.;
  entrypoint = "yamler:main";
  dependencies = (wasiPythonPkgs: [ wasiPythonPkgs.pyyaml ]);
  inputs = {
    yamlkey = { type = "string"; };
    yaml = { type = "string"; };
  };
  outputs = {
    utputt = {
      type = "string";
    };
  };
}
