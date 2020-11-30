{ base, pkgs }:
let
  name = "wasi-python-shims";
  stdenv = pkgs.pkgsCross.wasi32.clang11Stdenv;
  overriddenPackage = (base.languages.rust.mkPackage.override { inherit stdenv; });

  package = overriddenPackage {
    inherit name;
    src = ./.;
    defaultTarget = "wasm32-wasi";
    targets = [ "wasm32-wasi" ];

    doCrossCheck = true;

    # llvm is needed for dsymutil which something uses
    # when running cargo test
    checkInputs = [ pkgs.llvmPackages_11.llvm pkgs.wasmtime ];
  };

  newPackage = package.overrideAttrs (
    oldAttrs: {
      installPhase = ''
        ${oldAttrs.installPhase}
        mkdir -p $out/lib $out/include
        cp target/wasm32-wasi/release/libwasi_python_shims.a $out/lib
        cp wasi_python_shims.h $out/include
      '';

      # need the -crt-static to only be used when the library is intended
      # to be used from other C libraries, not for tests etc.
      buildPhase = ''
        (
          export RUSTFLAGS="${oldAttrs.RUSTFLAGS or ""} -C target-feature=-crt-static"
          ${oldAttrs.buildPhase}
        )
      '';

      checkPhase = ''
        ${oldAttrs.checkPhase}
        make check
      '';
    }
  );
in
base.mkComponent { inherit name; package = newPackage; deployment = { }; }
