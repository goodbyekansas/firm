{ base }:
base.languages.rust.mkLibrary {
  name = "windows-install";
  src = ./.;

  crossTargets = {
    includeNative = false;
    windows = { };
  };
}

