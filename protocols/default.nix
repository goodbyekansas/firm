project:
rec {
  rustProtoCompiler = project.declareComponent ./rust/compiler/compiler.nix { };

  python = project.declareComponent ./python/python.nix { };
  rust = rec {
    withServices = project.declareComponent ./rust/rust.nix {
      rustProtoCompiler = rustProtoCompiler.package;
      includeServices = true;
    };

    onlyMessages = project.declareComponent ./rust/rust.nix {
      rustProtoCompiler = rustProtoCompiler.package;
      includeServices = false;
    };

    testHelpers = { protocols ? onlyMessages }:
      project.declareComponent ./rust/test_macros/test_macros.nix {
      inherit protocols;
    };
  };
}
