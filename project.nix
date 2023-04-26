{ nedryland
, nedryglot
, pkgs
, oxalica ? (import ./dependencies.nix).oxalica
, ...
}:
let
  nedryland' = nedryland { inherit pkgs; };
  nedryglot' = nedryglot { };
in
nedryland'.mkProject rec {
  name = "firm";
  version = "6.0.0";
  baseExtensions = [
    nedryglot'.languages
    (import ./extensions/nedryland/rust.nix oxalica)
    nedryglot'.protobuf
    ./extensions/nedryland/function.nix
    ./extensions/nedryland/runtime.nix
  ];

  checks = nedryland'.checks;

  components = { callFile }: rec {
    protocols = callFile ./protocols/protocols.nix { };

    firmTypes = {
      rust = rec {
        withoutServices = callFile ./libraries/rust/firm-types/firm-types.nix {
          protocols = protocols.withoutServices.rust;
        };
        withServices = withoutServices.override {
          protocols = protocols.withServices.rust;
        };
      };
      python = callFile ./libraries/python/firm-types/firm-types.nix {
        protocols = protocols.withoutServices.python;
      };
    };

    wasiPythonShims = callFile ./services/avery/runtimes/python/wasi-python-shims/wasi-python-shims.nix { };
    wasiPython = callFile ./services/avery/runtimes/python/wasi-python.nix {
      pythonSource = builtins.fetchTarball {
        url = "https://github.com/goodbyekansas/cpython/archive/3c8f8d80d2f71197b702da6540af4f97e8fadafe.tar.gz";
        sha256 = "1ify5di2fmxrk6ss22vskvvp63r92xm0g4d21y29slxp74vr4kmx";
      };
    };

    runtimes = {
      python = callFile ./services/avery/runtimes/python/python.nix {
        firmTypes = firmTypes.rust.withoutServices;
      };
    };

    avery = callFile ./services/avery/avery.nix {
      types = firmTypes.rust.withServices;
    };

    bendini = callFile ./clients/bendini/bendini.nix {
      types = firmTypes.rust.withServices;
    };

    capellini = callFile ./clients/capellini/capellini.nix { };

    firmRust = callFile ./libraries/rust/firm-rust/firm-rust.nix {
      types = firmTypes.rust.withoutServices;
    };

    quinn = callFile ./services/quinn/quinn.nix {
      types = firmTypes.rust.withServices;
    };

    lomax = callFile ./services/lomax/lomax.nix {
      types = firmTypes.rust.withServices;
    };

    tonicMiddleware = callFile ./libraries/rust/tonic-middleware/tonic-middleware.nix {
      protocols = protocols.withServices.rust; # This brings tonic which we will need. A bit hard to see.
    };

    windowsInstall = callFile ./libraries/rust/windows-install/windows-install.nix { };

    firmWindowsInstaller = callFile ./clients/firm-windows-installer/firm-windows-installer.nix { inherit version; };
  };

  extraShells = { callFile }: {
    release = callFile ./extensions/shells/release.nix { };
  };
}
