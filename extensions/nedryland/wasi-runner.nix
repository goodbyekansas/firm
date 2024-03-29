{ gdb, writeShellScript, wasmtime, makeSetupHook }:
makeSetupHook
{
  name = "wasi-runner-hook";
  substitutions = {
    runner =
      # run the tests through virtual vm, create a temp directory and map it to the vm
      writeShellScript
        "runner.sh"
        ''
          temp_dir=$(mktemp -d)
          command="${wasmtime}/bin/wasmtime run"
          args="--env=RUST_TEST_NOCAPTURE=1 --disable-cache --mapdir=::$temp_dir"
          if [ -n "$RUST_DEBUG" ]; then
            args="-g $args"
            command="${gdb}/bin/gdb --args $command"
          fi
          command $command $args "$@"
          exit_code=$?
          rm -rf $temp_dir
          exit $exit_code
        '';
  };
}
  (builtins.toFile "wasi-runner-hook" ''
    export CARGO_TARGET_WASM32_WASI_RUNNER=@runner@
  '')
