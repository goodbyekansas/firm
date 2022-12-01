{ base }:
base.mkComponent rec{
  name = "protocols";
  source = ./.;
  withoutServices = base.languages.protobuf.mkModule {
    name = "firm-protocols";
    src = source;
    version = "1.0.0";
    languages = [ base.languages.rust base.languages.python ];
    includeServices = false;
  };
  withServices = base.languages.protobuf.mkModule {
    name = "firm-protocols";
    src = source;
    version = "1.0.0";
    languages = [ base.languages.rust base.languages.python ];
    includeServices = true;
  };
}
