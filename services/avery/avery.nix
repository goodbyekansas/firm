{ base
, protocols
, tonicMiddleware
, xcbuild ? null
, pkg-config
, lib
, stdenv
, system
, systemd
, libraries
}:
base.languages.rust.nativeTools.mkService rec {
  name = "avery";
  src = ./.;
  srcExclude = [
    (path: type: (type == "regular" && baseNameOf path == "CHANGELOG.md"))
  ];

  doCrossCheck = false; # https://github.com/tokio-rs/tokio/issues/4781 fixed in wine 7.13

  buildInputs = [
    protocols.withServices.rust
    tonicMiddleware
    libraries.runtime
    libraries.registry
    libraries.function
  ];

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

}
