{ stdenv
, base
, types
, tonicMiddleware
, xcbuild ? null
, pkg-config
, lib
, system
, systemd
}:
base.languages.rust.nativeTools.mkService {
  name = "lomax";
  src = ./.;

  buildInputs = [ types tonicMiddleware ];

  nativeBuildInputs = [ pkg-config ]
    ++ lib.optional stdenv.hostPlatform.isDarwin xcbuild;

  shellInputs = lib.optional (builtins.elem system lib.systems.doubles.linux) systemd;

  shellCommands = lib.optionalAttrs (builtins.elem system lib.systems.doubles.linux) {
    testSocketActivation = {
      script = ''
        systemd-socket-activate -l ''${1:-0} target/debug/lomax
      '';
      args = "[PORT]";
      description = "Run lomax with systemd socket activation";
    };
  };
}
