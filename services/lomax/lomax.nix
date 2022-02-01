{ stdenv
, base
, types
, tonicMiddleware
, xcbuild ? null
, pkg-config
, lib
, systemd
}:
base.languages.rust.mkService rec {
  name = "lomax";
  src = ./.;

  buildInputs = [ types tonicMiddleware ];

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
      inherit buildInputs;
    };
  };
}
