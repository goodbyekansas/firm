{ base, pkgs }: # TODO: Make one deriv for the C output and another for rust
let
  name = "wasi-python-shims";
  stdenv = pkgs.pkgsCross.wasi32.clang11Stdenv;
  overriddenMkPackage = (base.languages.rust.mkPackage.override { inherit stdenv; });

  package = overriddenMkPackage {
    inherit name;
    src = ./.;
    defaultTarget = "wasm32-wasi";
    targets = [ "wasm32-wasi" ];

    doCrossCheck = true;
    useNightly = "2021-03-04";

    # llvm is needed for dsymutil which something uses
    # when building in debug
    nativeBuildInputs = pkgs.lib.optional pkgs.stdenv.isDarwin pkgs.llvmPackages_11.llvm;

    checkInputs = [ pkgs.wasmtime ];
    shellInputs = [ pkgs.bear ];
  };

  libraryPackage = base.languages.rust.toLibrary package;

  newPackage = libraryPackage.overrideAttrs (
    oldAttrs: {
      installPhase = ''
        ${oldAttrs.installPhase}
        mkdir -p $out/lib $out/include
        cp target/wasm32-wasi/release/libwasi_python_shims.a $out/lib
        cp $(cargo run --release header) $out/include
      '';

      # need the -crt-static to only be used when the library is intended
      # to be used from other C libraries, not for tests etc.
      buildPhase = ''
        (
          export RUSTFLAGS="$RUSTFLAGS -C target-feature=-crt-static"
          ${oldAttrs.buildPhase}
          cargo build --release
        )
      '';

      # TODO: remove the "clean" target when
      # https://github.com/goodbyekansas/nedryland/issues/143
      # is done. Then the test exe will be ignored by gitignore
      checkPhase = ''
        ${oldAttrs.checkPhase}
        make clean check
      '';
    }
  );
in
base.mkComponent { inherit name; package = newPackage; deployment = { }; }
