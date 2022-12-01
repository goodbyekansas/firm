{ base
, types
, tonicMiddleware
, xcbuild ? null
, pkg-config
, lib
, stdenv
, system
, systemd
}:
(base.languages.rust.nativeTools.mkService rec {
  name = "avery";
  src = ./.;
  srcExclude = [
    (path: type: (type == "directory" && baseNameOf path == "runtimes"))
    (path: type: (type == "regular" && baseNameOf path == "avery-with-runtimes.nix"))
    (path: type: (type == "regular" && baseNameOf path == "CHANGELOG.md"))
  ];

  doCrossCheck = false; # https://github.com/tokio-rs/tokio/issues/4781 fixed in wine 7.13

  buildInputs = [ types tonicMiddleware ];

  nativeBuildInputs = [ pkg-config ]
    ++ lib.optional stdenv.buildPlatform.isDarwin xcbuild;

  shellInputs = lib.optional (builtins.elem system lib.systems.doubles.linux) systemd;

  shellCommands = lib.optionalAttrs (builtins.elem system lib.systems.doubles.linux) {
    testSocketActivation = {
      script = ''
        cargo build
        systemd-socket-activate -l "/tmp/avery-dev.sock" "target/debug/avery"
      '';
    };
  };

}).overrideAttrs (avery: {
  withRuntimes = base.callFile ./avery-with-runtimes.nix {
    inherit avery;
  };

  withDefaultRuntimes = (base.callFile ./avery-with-runtimes.nix {
    inherit avery;
  }) { };
})
