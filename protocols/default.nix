project:
rec {
  rustProtoCompiler = project.declareComponent ./rust/compiler/compiler.nix { };

  python = project.declareComponent ./python/python.nix {
    protoSources = ./proto;
  };

  rust = rec {
    withServices = project.declareComponent ./rust/rust.nix {
      rustProtoCompiler = rustProtoCompiler.package;
      includeServices = true;
      protoSources = ./proto;
    };

    onlyMessages = project.declareComponent ./rust/rust.nix {
      rustProtoCompiler = rustProtoCompiler.package;
      includeServices = false;
      protoSources = ./proto;
    };

  };
}
