{ base, types, tonicMiddleware }:
base.languages.rust.mkClient rec {
  name = "bendini";
  src = ./.;
  buildInputs = [ types.package tonicMiddleware.package ];

  crossTargets = {
    windows = { };
  };

  shellHook = ''
    bendiniLocal() {
      command cargo run --quiet -- --host unix://localhost"$XDG_RUNTIME_DIR"/avery.sock --auth-host unix://localhost"$XDG_RUNTIME_DIR"/avery.sock "$@"
    }

    bendiniLocalAuth() {
      command cargo run --quiet -- --auth-host unix://localhost"$XDG_RUNTIME_DIR"/avery.sock "$@"
    }

    echo -e "Use \e[32;1mbendiniLocal\e[0m to run bendini against a development version of avery \e[36;1m(both auth and functions)\e[0m"
    echo -e "Use \e[32;1mbendiniLocalAuth\e[0m to run bendini against a development version of avery \e[36;1m(auth only)\e[0m"
  '';
}
