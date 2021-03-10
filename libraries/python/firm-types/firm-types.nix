{ pkgs, base, protocols }:

base.languages.python.mkLibrary rec{
  name = "firm-types";
  version = "0.1.0";
  src = ./.;
  pythonVersion = pkgs.python3;
  propagatedBuildInputs = (pythonPkgs: [
    protocols.package
  ]);
}
