{ base, pkgs, firmRust, firmTypes, avery, bendini, wasiPython, wasiPythonShims }:
let
  examples = {
    hello = base.callFile ./examples/hello/hello.nix { };
    firmApi = base.callFile ./examples/firm-api/firm-api.nix { };
    firmApiError = base.callFile ./examples/firm-api/firm-api-error.nix { };
    networking = base.callFile ./examples/networking/networking.nix { };
    yamler = base.callFile ./examples/yamler/yamler.nix { };
  };

  fileSystemImage = (pkgs.linkFarm "python-runner-fs-image" [{
    name = "lib";
    path = "${wasiPython.package}/lib/python3.8";
  }]);

  zlib = pkgs.pkgsCross.wasi32.zlib.override {
    stdenv = pkgs.pkgsCross.wasi32.clang12Stdenv;
  };

  exampleRunFunctions = ''
    runApiExample()
    {(
      set +e
      echo "Running firmApi example"
      command cargo run firmApi \
      -i str_input="hej" \
      -i int_input=5 \
      -i float_input=3.15 \
      -i bool_input=false \
      -i bytes_input=32 \
      -i str_list_input="[heffaklump 🐙]" \
      -i int_list_input="[1 -14 4444]" \
      -i bool_list_input="[true false true false]" \
      -i float_list_input="[0.1 0.2 123123123.2]" 2>&1 | sed "s/^/  [firmApi] /"
      exit_code=$?
      if [ $exit_code -ne 0 ]; then
        return $exit_code
      fi

      echo "Running firmApiError example"
      command cargo run firmApiError 2>&1 | sed "s/^/  [firmApiError] /"
      exit_code=$?
      # Exit code 16 by convention. It means bendini ran into a function runtime error.
      # This is what we want here so return 0 instead
      if [ $exit_code -eq 16 ]; then
        return 0
      fi

      return $exit_code
    )}

    runNetExample() {
      echo "Starting netcat on port 3333..."
      nc -l -k 3333 2>&1 | sed "s/^/  [network] /" &
      command cargo run networking -i port=3333
      kill %1 && wait %1
    }

    runDepsExample() {
      echo "Running yaml dependency example"
      command cargo run yamler \
      -i yaml="$1" \
      -i yamlkey="$2" 2>&1 | sed "s/^/  [yamler] /"
    }

  '';

  # Python comes with an (env hook)[https://github.com/NixOS/nixpkgs/blob/f46390a8733096606e1ff18f393609769fa72d39/pkgs/development/interpreters/python/cpython/default.nix#L161]
  # which sets _PYTHON_SYSCONFIGDATA_NAME to the host platform, which pyo3 uses to find sysconfigdata.
  # This is incorrect for us because pyo3 will then think it should look for the host plaform instead of wasi/wasm.
  pythonWithoutHook = ((pkgs.python3).overrideAttrs (_: { postFixup = ""; }));
in
base.mkRuntime {
  inherit examples fileSystemImage;

  name = "python-runtime";
  runtimeName = "python";
  src = ./.;

  testFeatures = [ "mock" ];
  doCrossCheck = true;
  exposeRunnerInChecks = true;
  extraChecks = ''
    ${exampleRunFunctions}
    export CARGO_TARGET_WASM32_WASI_RUNNER=runtime-runner
    temp_dir=$(mktemp -d)
    export HOME=$temp_dir
    cargo run hello
    runApiExample
    runDepsExample "sune: suna" "sune"
  '';

  nativeBuildInputs = [ pythonWithoutHook ];
  checkInputs = [ avery bendini ];

  buildInputs = [ firmRust.wasi firmTypes wasiPythonShims zlib ];
  shellInputs = [ pkgs.netcat ];
  shellHook = exampleRunFunctions;
  RUSTFLAGS = "-Ctarget-feature=-crt-static -Clink-args=-lwasi-emulated-signal -Clink-args=-lwasi-emulated-process-clocks -lz";

  # Environment variable used to skip a step in the build process for wasi shims.
  # We build directly with the library so we do not need any C bindgens.
  # Setting it to anything will skip it.
  WASI_PYTHON_SHIMS_SKIP_C_BINDGEN = "Yes";

  PYO3_CROSS_LIB_DIR = "${wasiPython.package}/lib/";
}
