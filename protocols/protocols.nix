{ base, python3 }:
rec{
  source = ./.;
  withoutServices = base.languages.protobuf.mkModule {
    name = "firm-protocols";
    src = source;
    version = "1.0.0";
    languages = [ base.languages.rust base.languages.python ];
    pythonVersion = python3;
    includeServices = false;
  };
  withServices = base.languages.protobuf.mkModule {
    name = "firm-protocols";
    src = source;
    version = "1.0.0";
    languages = [ base.languages.rust base.languages.python ];
    pythonVersion = python3;
    includeServices = true;
  };
}
