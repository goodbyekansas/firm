{ base }:
(base.languages.rust.nativeTools.override {
  crossTargets.rust = base.languages.rust.nativeTools.crossTargets.windows;
}).mkLibrary {
  name = "windows-install";
  src = ./.;
  defaultTarget = "windows";
}
