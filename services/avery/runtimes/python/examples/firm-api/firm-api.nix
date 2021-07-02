{ base, pkgs }:
base.languages.python.mkFunction {
  name = "firm-api";
  version = "1.0.0";
  src = ./.;
  entrypoint = "firm_api:main";

  attachments = {
    data = "much_data.dat";
    compressed_data = "compressed_data.tar.gz";
  };

  inputs = {
    str_input = {
      type = "string";
    };
    int_input = {
      type = "int";
    };
    float_input = {
      type = "float";
    };
    bool_input = {
      type = "bool";
    };
    bytes_input = {
      type = "bytes";
    };
    str_list_input = {
      type = "string";
    };
    int_list_input = {
      type = "int";
    };
    float_list_input = {
      type = "float";
    };
    bool_list_input = {
      type = "bool";
    };
  };
  outputs = {
    str_output = {
      type = "string";
    };
    int_output = {
      type = "int";
    };
    float_output = {
      type = "float";
    };
    bool_output = {
      type = "bool";
    };
    bytes_output = {
      type = "bytes";
    };
  };
}
