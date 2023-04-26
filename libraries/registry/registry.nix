{ base }:
base.languages.rust.nativeTools.mkLibrary {
  name = "libregistry";
  src = ./.;
}
