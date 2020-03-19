import (import ./nedryland.nix)
{
    projectName = "firm";
    defaultConfigFile = ./firm.json;
    componentPaths = [
        ./services/avery/avery.nix
        ./functions/start-maya/start-maya.nix
        ./clients/bendini/bendini.nix
        ./clients/lomax/lomax.nix
    ];
}
