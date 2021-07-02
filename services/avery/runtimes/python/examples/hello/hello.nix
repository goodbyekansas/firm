{ base, pkgs }:
base.languages.python.mkFunction {
  name = "hello";
  version = "1.0.0";
  src = ./.;
  entrypoint = "hello:main";
}
