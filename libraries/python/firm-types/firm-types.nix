{ pkgs, base, protocols }:

base.languages.python.mkLibrary rec{
  name = "firm-types";
  version = "1.0.0";
  src = ./.;
  pythonVersion = pkgs.python3;
  propagatedBuildInputs = (_: [
    protocols.package
  ]);
}
