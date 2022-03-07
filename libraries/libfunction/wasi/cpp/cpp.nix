{ base
, pkgsCross
, libfunction
, wasmtime
, wasi-clangd
, python38Packages
, doxygen
, buildPackages
, valgrind
}:

base.mkLibrary rec {
  name = "firm-function-cpp";
  wasi = base.mkDerivation {
    inherit name;
    src = ./.;
    stdenv = pkgsCross.wasi32.clang12Stdenv;

    nativeBuildInputs = [ buildPackages.stdenv.cc ];
    buildInputs = [ libfunction.wasi.c.wasi ];

    checkInputs = [ valgrind wasmtime ];

    shellInputs = [ wasi-clangd python38Packages.compiledb ];

    doCrossCheck = true;
    checkPhase = ''
      echo ""
      echo "💄 Checking formatting on C++ files..."

      if clang-format -Werror -n ./**/*.hh ./**/*.cpp; then
        echo "💄 Perfectly formatted!"
      fi

      echo ""
      echo "💾 Running build platform C++ tests with Valgrind..."
      make valgrind
    '';

    crossCheckPhase = ''
      echo ""
      echo "🐺 Running WASI C tests..."
      make check
    '';

    shellHook = ''
      check() {
        eval "$checkPhase"
      }
    '';

    CXX_FOR_BUILD = "${buildPackages.stdenv.cc.targetPrefix}c++";
    LIBSTDCXX_FOR_BUILD = buildPackages.stdenv.cc.cc.lib;
    makeFlags = [ "prefix=${placeholder "out"}" ];
  };

  docs.api = base.mkDerivation {
    src = ./.;
    name = "${name}-api-reference";
    nativeBuildInputs = [ doxygen ];

    buildPhase = ''
      doxygen
    '';

    installPhase = ''
      mkdir -p $out/share/doc/firm-function-cpp/api
      cp -r generated-docs/html/. $out/share/doc/firm-function-cpp/api
    '';
  };
}
