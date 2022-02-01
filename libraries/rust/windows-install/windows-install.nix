{ base }:
base.languages.rust.mkLibrary {
  name = "windows-install";
  src = ./.;
  defaultTarget = "windows";
}

