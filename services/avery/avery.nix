{ base
, types
, tonicMiddleware
, xcbuild ? null
, pkgsCross ? null
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
  ];

  buildInputs = [ types.package tonicMiddleware.package ];

  nativeBuildInputs = [ pkg-config ]
    ++ lib.optional stdenv.hostPlatform.isDarwin xcbuild;

  shellInputs = [
    systemd
  ];

  crossTargets = {
    windows = {
      buildInputs = buildInputs ++ [ pkgsCross.mingwW64.windows.pthreads ];
    };
  };

  shellHook = ''
    testSocketActivation() {
      cargo build
      systemd-socket-activate -l "/tmp/avery-dev.sock" "target/debug/avery"
    }
  '';
}).overrideAttrs (attrs: {
  withRuntimes = base.callFile ./avery-with-runtimes.nix {
    avery = attrs.package;
  };

  withDefaultRuntimes = (base.callFile ./avery-with-runtimes.nix {
    avery = attrs.package;
  }) { };
})
