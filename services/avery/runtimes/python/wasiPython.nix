{ base, pkgs, pythonSource, overrideCC, wasiPythonShims }:
let
  zlib = pkgs.pkgsCross.wasi32.zlib.override {
    stdenv = pkgs.pkgsCross.wasi32.clang11Stdenv;
  };
in
base.mkComponent {
  # we need clang 11 for being able to debug and print variables
  name = "wasi-python38";
  package = pkgs.pkgsCross.wasi32.clang11Stdenv.mkDerivation {
    name = "wasi-python38";
    src = pythonSource;
    buildInputs = [ wasiPythonShims.package zlib ];
    nativeBuildInputs = [ pkgs.autoreconfHook pkgs.pkg-config pkgs.python38 ];
    # gdb is needed for JIT debugging with lldb. I know, it's a weird relationship they have together.
    shellInputs = [ pkgs.bear pkgs.lldb_11 pkgs.gdb pkgs.wasmtime ];
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
        bear bash -c build
      }

      pythonTests() {
        command wasmtime run python.wasm --dir . -- -m test
      }

      configure() {
        configurePhase
      }

      build() {
        buildPhase
      }

      rebuild() {
        make clean
        build
      }

      debug_program() {
        application="$1"
        debugger=''${2:-lldb}
        shift 2

        if [ $debugger ==  "lldb" ]; then
          command lldb \
            -O 'settings set plugin.jit-loader.gdb.enable on' \
            -O "command regex pp 's/(.+)/p __vmctx->set(),%1/'" \
            -- wasmtime  run \
              -g --opt-level 0 --dir=. "$application" -- "$@"
        elif [ $debugger == "gdb" ]; then
          command gdb -x ${./wasiprint.gdb} -ex "set breakpoint pending on" --args wasmtime run -g --opt-level 0 --dir=. "$application" -- "$@"
        else
          echo "Unsupported debugger 🧂🦨"
          return 1
        fi
      }

      debug() {
        debug_program "python.wasm" "$@"
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
