{ base, wabt, wasmtime, python38Packages, libfunction, doxygen, wasi-clangd, lldb_12, valgrind, linuxPackages }:
(base.languages.rust.mkLibrary {
  name = "firm-function-c";
  src = ./.;

  defaultTarget = "wasi";

  buildInputs = [ libfunction.wasi.rust ];

  shellInputs = [ wabt python38Packages.compiledb wasi-clangd lldb_12 ];
  checkInputs = [ wasmtime valgrind linuxPackages.perf ];

  doCrossCheck = true;
}).overrideAttrs (libfunc:
  {
    # replace rust docs (which will not be useful) with docs generated from the
    # hand-crafted C headers instead
    docs.api = base.mkDerivation {
      src = libfunc.wasi.src;
      name = "${libfunc.wasi.name}-api-reference";
      nativeBuildInputs = [ doxygen ];

      buildPhase = ''
        doxygen
      '';

      installPhase = ''
        mkdir -p $out/share/doc/firm-function-c/api
        cp -r generated-docs/html/. $out/share/doc/firm-function-c/api
      '';
    };

    wasi = libfunc.wasi.overrideAttrs (w:
      {
        # do not want the C runtime linked statically since we are creating a lib for C
        RUSTFLAGS = "-C target-feature=-crt-static ${w.RUSTFLAGS or ""}";

        # need to set a buildPhase here since the standard rust lib one does not actually build
        buildPhase = ''
          cargo build --release
        '';

        shellHook = ''
          format() {
            cargo fmt
            clang-format -i **/*.h **/*.c
          }

          ${w.shellHook or ""}
        '';

        checkPhase = ''
          ${w.checkPhase or ""}
          echo ""
          echo "💄 Checking formatting on C files..."
          clang-format -Werror -n **/*.h **/*.c

          if [ $? -eq 0 ]; then
            echo "💄 Perfectly formatted!"
          fi

          echo ""
          echo "💽 Running build platform C tests with Valgrind..."
          make valgrind
        '';

        crossCheckPhase = ''
          ${w.crossCheckPhase or ""}
          echo ""
          echo "🦊 Running WASI C tests..."
          make check
        '';

        installPhase = ''
          mkdir -p $out/lib $out/include/firm/types
          cp target/wasm32-wasi/release/libfirm_function.a $out/lib
          cp src/function.h $out/include/firm/function.h
          cp src/types/*.h $out/include/firm/types
        '';
      });
  })
