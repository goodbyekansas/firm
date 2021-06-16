{ stdenv
, base
, types
, tonicMiddleware
, xcbuild ? null
, pkgsCross ? null
, pkg-config
, lib
}:
base.languages.rust.mkService rec {
  name = "lomax";
  src = ./.;

  buildInputs = [ types.package tonicMiddleware.package ];
  nativeBuildInputs = [ pkg-config ]
    ++ lib.optional stdenv.hostPlatform.isDarwin xcbuild;

  crossTargets = {
    windows = {
      buildInputs = buildInputs ++ [ pkgsCross.mingwW64.windows.pthreads ];
    };
  };

}
