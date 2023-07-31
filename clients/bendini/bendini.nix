{ base, types, tonicMiddleware }:
base.languages.rust.nativeTools.mkClient {
  name = "bendini";
  src = ./.;
  buildInputs = [ types tonicMiddleware ];

  shellCommands = {
    bendiniLocal = {
      script = ''cargo run --quiet -- --host unix://localhost"$XDG_RUNTIME_DIR"/avery.sock --auth-host unix://localhost"$XDG_RUNTIME_DIR"/avery.sock "$@"'';
      description = "Run bendini against a development version of avery (both auth and functions)";
    };

    bendiniLocalAuth = {
      script = ''cargo run --quiet -- --auth-host unix://localhost"$XDG_RUNTIME_DIR"/avery.sock "$@"'';
      description = "Run bendini against a development version of avery (auth only)";
    };
  };
}
