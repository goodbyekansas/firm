{ base }:
let
  pyYamlTypes = py: py.buildPythonPackage rec {
    pname = "types-PyYAML";
    version = "6.0.12.2";

    src = py.fetchPypi {
      inherit pname version;
      sha256 = "sha256-aECBmHHJLe6+aiBn+4AMEbigY2MutOPnVZFOerNgToM=";
    };

    preBuild = ''
      export HOME=$PWD
    '';
  };

in
base.languages.python.mkFunction {
  name = "yamler";
  version = "1.0.0";
  src = ./.;
  entrypoint = "yamler:main";
  dependencies = (wasiPythonPkgs: [ wasiPythonPkgs.pyyaml ]);
  hostCheckDependencies = (pypkgs: [ (pyYamlTypes pypkgs) ]);
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
