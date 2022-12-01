{ base, pkgs, }:
((base.languages.rust.nightly.withWasi.override {
  crossTargets.rust = base.languages.rust.nightly.withWasi.crossTargets.wasi;
}).mkLibrary {
  name = "wasi-python-shims";
  src = ./.;

  checkInputs = [ pkgs.wasmtime ];
  shellInputs = [ pkgs.bear ];

}).overrideAttrs (old: {
  c = old.rust.overrideAttrs (rust: {
    RUSTFLAGS = "${rust.RUSTFLAGS or ""} -Ctarget-feature=-crt-static";
    buildPhase = ''
      runHook preBuild
      cargo build --release
      runHook postBuild  
    '';

    installPhase = ''
      runHook preInstall
      mkdir -p $out/lib $out/include
      cp target/wasm32-wasi/release/libwasi_python_shims.a $out/lib
      # Inputting the build target platform manually to not run wasi here.
      cp $(cargo run --target ${base.languages.rust.toRustTarget pkgs.stdenv.buildPlatform} --release header) $out/include
      runHook postBuild
    '';

    postCheck = "make check";
  });
})
