{ stdenv
, base
, types
, tonicMiddleware
, xcbuild ? null
, pkgsCross ? null
, pkg-config
, lib
, systemd
}:
base.languages.rust.mkService rec {
  name = "lomax";
  src = ./.;

  buildInputs = [ types.package tonicMiddleware.package ];

  nativeBuildInputs = [ pkg-config ]
    ++ lib.optional stdenv.hostPlatform.isDarwin xcbuild;

  shellInputs = [ systemd ];

  shellHook = ''
    testSocketActivation() {
      systemd-socket-activate -l 1939 target/debug/lomax
    }
  '';

  crossTargets = {
    windows = {
      buildInputs = buildInputs ++ [ pkgsCross.mingwW64.windows.pthreads ];
    };
  };
}
