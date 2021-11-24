{ nedrylandOverride ? null }:
let
  sources = import ./nix/sources.nix;
  nedryland = (if nedrylandOverride == null then (import sources.nedryland { }) else nedrylandOverride);
in
nedryland.mkProject rec {
  name = "firm";
  version = "1.2.1";
  baseExtensions = [
    ./extensions/nedryland/function.nix
    ./extensions/nedryland/runtime.nix
  ];
  ci = nedryland.ci;

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
    wasiPython = callFile ./services/avery/runtimes/python/wasiPython.nix {
      pythonSource = sources.python;
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

    windowsInstaller = callFile ./clients/windows-installer/windows-installer.nix { inherit version; };
  };

  extraShells = { callFile }: {
    release = callFile ./extensions/shells/release.nix { inherit components; };
  };
}
