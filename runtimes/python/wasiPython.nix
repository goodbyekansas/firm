{ base, pkgs, pythonSource, overrideCC, wasiPythonShims }:
base.mkComponent {
  # we need clang 11 for being able to debug and print variables
  package = pkgs.pkgsCross.wasi32.clang11Stdenv.mkDerivation {
    name = "wasi-python38";
    src = pythonSource;
    buildInputs = [ wasiPythonShims.package ];
    nativeBuildInputs = [ pkgs.wasmtime pkgs.autoreconfHook pkgs.pkg-config pkgs.python38 ];
    shellInputs = [ pkgs.bear pkgs.lldb_11 pkgs.gdb ]; # gdb is needed for JIT debugging, I think?
    configureFlags = [
      "--disable-ipv6"
      "--with-suffix=.wasm"
      "--without-ensurepip" # TODO: Maybe we want this?
      "ac_cv_file__dev_ptmx=no"
      "ac_cv_file__dev_ptc=no"
      "ac_cv_wasmthread=yes"
    ];

    preConfigure = ''
      configureFlagsArray=(
        "CPPFLAGS=-D_WASI_EMULATED_SIGNAL -fPIC"
        "LDFLAGS=-lwasi-emulated-signal -Wl,--stack-first -z stack-size=2097152" # 2 MiB stack 🥞
      )
    '';

    makeFlags = [
      "LINKFORSHARED= "
      "LDFLAGS=-lwasi_python_shims"
    ];

    checkPhase = ''
      echo "TODO: Run the python tests here?"
    '';

    shellHook = ''
      configureFlags+=" --with-pydebug"

      # We run bear make to create files useful by LSP
      bearMake() {
        command make clean
        command bear make "$@"
      }

      configure() {
        configurePhase
      }

      build() {
        buildPhase
      }

      debug() {
        debugger=''${1:-lldb}
        if [ $debugger ==  "lldb" ]; then
          command lldb \
            -O 'settings set plugin.jit-loader.gdb.enable on' \
            -O "command regex pp 's/(.+)/p __vmctx->set(),%1/'" \
            -- wasmtime  run \
              -g --opt-level 0 --dir=. python.wasm
        elif [ $debugger == "gdb" ]; then
          command gdb -x ${./wasiprint.gdb} -ex "set breakpoint pending on" --args wasmtime run -g --opt-level 0 --dir=. python.wasm
        else
          echo "Unsupported debugger 🧂🦨"
          return 1
        fi
      }

      run() {
        # Consider where we put pythonthreaddebug
        command wasmtime run python.wasm --dir . -- "$@"
      }

      updateAutotools() {
        updateAutotoolsGnuConfigScriptsPhase
      }

      reconf() {
        autoreconfPhase
      }

      cleanRun() {
        make clean
        build
        run
      }
    '';
    #wasiLibC = pkgs.wasilibc.outPath; TODO see if we can make this path a variable without having to turn on unsupported systems
  };
  path = pythonSource;
}
