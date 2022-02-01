{ base
, types
, tonicMiddleware
, xcbuild ? null
, pkg-config
, lib
, stdenv
, systemd
}:
(base.languages.rust.mkService rec {
  name = "avery";
  src = ./.;
  srcExclude = [
    (path: type: (type == "directory" && baseNameOf path == "runtimes"))
    (path: type: (type == "regular" && baseNameOf path == "avery-with-runtimes.nix"))
    (path: type: (type == "regular" && baseNameOf path == "CHANGELOG.md"))
  ];

  buildInputs = [ types tonicMiddleware ];

  nativeBuildInputs = [ pkg-config ]
    ++ lib.optional stdenv.hostPlatform.isDarwin xcbuild;

  shellInputs = [
    systemd
  ];

  crossTargets = {
    windows = {
      inherit buildInputs;
    };
  };

  shellHook = ''
    testSocketActivation() {
      cargo build
      systemd-socket-activate -l "/tmp/avery-dev.sock" "target/debug/avery"
    }
  '';

}).overrideAttrs (avery: {
  withRuntimes = base.callFile ./avery-with-runtimes.nix {
    inherit avery;
  };

  withDefaultRuntimes = (base.callFile ./avery-with-runtimes.nix {
    inherit avery;
  }) { };
})
