import (import ./nedryland.nix)
{
  name = "firm";
  configFile = ./firm.json;
  componentInitFn =
    { nedryland }:
    rec {
      avery = nedryland.declareComponent ./services/avery/avery.nix {};
      start-maya = nedryland.declareComponent ./functions/start-maya/start-maya.nix {};
      bendini = nedryland.declareComponent ./clients/bendini/bendini.nix {};
      lomax = nedryland.declareComponent ./clients/lomax/lomax.nix {};

      os-packaging = nedryland.declareComponent ./deployment/os-packaging.nix {
        dependencies = {
          linuxPackages = [
            avery
          ];

          windowsPackages = [
          ];
        };
      };
    };
  }


