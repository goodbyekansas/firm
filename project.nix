{ nedrylandOverride ? null }:
let
  sources = import ./nix/sources.nix;
  nedryland = (if nedrylandOverride == null then (import sources.nedryland) else nedrylandOverride);
in
nedryland.mkProject {
  name = "firm";
  baseExtensions = [
    ./extensions/nedryland/function.nix
    ./extensions/nedryland/runtime.nix
  ];

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

    averyWithRuntimes = callFile ./services/avery/avery-with-runtimes.nix { };
    averyWithDefaultRuntimes = (callFile ./services/avery/avery-with-runtimes.nix { }) { };

    bendini = callFile ./clients/bendini/bendini.nix {
      types = firmTypes.rust.withServices;
    };

    firmRust = callFile ./libraries/rust/firm-rust/firm-rust.nix {
      types = firmTypes.rust.withoutServices;
    };

    osPackaging = callFile ./deployment/os-packaging.nix {
      linuxPackages = [
        avery
        bendini
      ];

      windowsPackages = [ ];
    };

    quinn = callFile ./services/quinn/quinn.nix {
      types = firmTypes.rust.withServices;
    };

    tonicMiddleware = callFile ./libraries/rust/tonic-middleware/tonic-middleware.nix {
      protocols = protocols.withServices.rust; # This brings tonic which we will need. A bit hard to see.
    };
  };
}
