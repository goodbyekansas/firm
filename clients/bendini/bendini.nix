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
}
