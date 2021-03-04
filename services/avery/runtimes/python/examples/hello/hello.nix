{ base, pkgs }:
base.languages.python.mkFunction {
  name = "hello";
  version = "0.1.0";
  src = ./.;
  entrypoint = "hello:main";
}
