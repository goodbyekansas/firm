{ base, types, tonicMiddleware, pkgsCross ? null }:
base.languages.rust.mkClient rec {
  name = "bendini";
  src = ./.;
  buildInputs = [ types.package tonicMiddleware.package ];

  crossTargets = {
    windows = {
      buildInputs = buildInputs ++ [ pkgsCross.mingwW64.windows.pthreads ];
    };
  };

  shellHook = ''
    bendiniLocal() {
      command cargo run -- --host "$XDG_RUNTIME_DIR"/avery.sock --auth-host "$XDG_RUNTIME_DIR"/avery.sock "$@"
    }

    bendiniLocalAuth() {
      command cargo run -- --auth-host "$XDG_RUNTIME_DIR"/avery.sock "$@"
    }

    echo -e "Use \e[32;1mbendiniLocal\e[0m to run bendini against a development version of avery \e[36;1m(both auth and functions)\e[0m"
    echo -e "Use \e[32;1mbendiniLocalAuth\e[0m to run bendini against a development version of avery \e[36;1m(auth only)\e[0m"
  '';
}
