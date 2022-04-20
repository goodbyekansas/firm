{ base, pkgs }: # TODO: Make one deriv for the C output and another for rust
(base.languages.rust.mkLibrary {
  name = "wasi-python-shims";
  src = ./.;
  defaultTarget = "wasi";

  doCrossCheck = true;
  useNightly = "2022-04-20";

  checkInputs = [ pkgs.wasmtime ];
  shellInputs = [ pkgs.bear ];
}
).overrideAttrs (
  oldAttrs: {
    package = oldAttrs.wasi.overrideAttrs (packageAttrs: {
      RUSTFLAGS = "${packageAttrs.RUSTFLAGS} -Ctarget-feature=-crt-static";
      installPhase = ''
        ${packageAttrs.installPhase}
        mkdir -p $out/lib $out/include
        cp target/wasm32-wasi/release/libwasi_python_shims.a $out/lib

        # Inputting the build target platform manually to not run wasi here.
        cp $(cargo run --target ${base.languages.rust.toRustTarget pkgs.stdenv.buildPlatform} --release header) $out/include
      '';

      buildPhase = ''
        cargo build --release
      '';

      checkPhase = ''
        ${packageAttrs.checkPhase}
        make check
      '';
    });
  }
)
