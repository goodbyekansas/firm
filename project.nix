let
  # the if statement here is to be able to switch to
  # a local dev version of nedryland
  # if you do not need this, just keep the else statement
  nedryland = import (
    if builtins.getEnv "NEDRYLAND_PATH" != "" then
      (./. + "/${builtins.getEnv "NEDRYLAND_PATH"}")
    else
      builtins.fetchGit {
        name = "nedryland";
        url = "git@github.com:goodbyekansas/nedryland.git";
        rev = "e2ffeeaf2e204e412e7f00f043c8692e1561cc8b";
      }
  );

  # declare our project
  project = nedryland.mkProject {
    name = "firm";
    configFile = ./firm.toml;
    baseExtensions = [
      (import ./nedryland/function.nix)
    ];
  };

  protocols = nedryland.importProject {
    name = "protocols";
    url = "git@github.com:goodbyekansas/protocols.git";
    rev = "b3fc3901b1e01495bb43569cad3eb9597ec872e0";
  };

  # declare the components of the project and their dependencies
  components = rec {
    wasiFunctionUtils = project.declareComponent ./utils/rust/gbk/gbk.nix {
      protocols = protocols.rustOnlyMessages;
    };
    avery = project.declareComponent ./services/avery/avery.nix {
      protocols = protocols.rustWithServices;
    };
    bendini = project.declareComponent ./clients/bendini/bendini.nix {
      protocols = protocols.rustWithServices;
    };
    lomax = project.declareComponent ./clients/lomax/lomax.nix {
      protocols = protocols.rustWithServices;
    };

    os-packaging = project.declareComponent ./deployment/os-packaging.nix {
      linuxPackages = [
        avery
      ];

      windowsPackages = [];
    };
  };
  capturedLomaxPackage = components.lomax.package;
  getFunctionDeployments = { components, endpoint ? "tcp://[::1]", port ? 1939 }: builtins.map (
    drv:
      drv {
        inherit endpoint port;
        lomax = capturedLomaxPackage;
      }
  ) (
    nedryland.getDeployments { inherit components; type = "function"; }
  );
in
  # create the build grid (accessed with nix-build, exposed through default.nix)
project.mkGrid {
  inherit components;
  deploy = rec {
    functions = getFunctionDeployments {
      inherit components;
    };

    local = [
      functions
    ];

    prod = [
      (
        getFunctionDeployments {
          inherit components;
          endpoint = "tcp://a.production.registry";
          port = 1337;
        }
      )
    ];
  };

  # create the project shells (accessed with nix-shell, exposed through shell.nix)
  extraShells = {};

  # Add functions and other things you want to re-export, making it publicly visible to users of firm.
  lib = {
    inherit getFunctionDeployments;
  };
} // protocols
