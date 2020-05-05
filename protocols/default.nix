project:
rec {
  rustProtoCompiler = project.declareComponent ./rust/compiler/compiler.nix {};

  python = project.declareComponent ./python/python.nix {};
  rust = {
    withServices = project.declareComponent ./rust/rust.nix {
      rustProtoCompiler = rustProtoCompiler.package;
      includeServices = true;
    };

    onlyMessages = project.declareComponent ./rust/rust.nix {
      rustProtoCompiler = rustProtoCompiler.package;
      includeServices = false;
    };
  };
}
