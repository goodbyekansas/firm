{ base, pkgs }:
base.languages.python.mkFunction {
  name = "firm-api-error";
  version = "1.0.0";
  src = ./.;
  entrypoint = "firm_api:main_with_error";
}
