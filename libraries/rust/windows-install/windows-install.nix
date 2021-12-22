{ base, pkgsCross }:
(base.languages.rust.mkLibrary {
  name = "windows-install";
  src = ./.;

  crossTargets = {
    windows = {
      buildInputs = [ pkgsCross.mingwW64.windows.pthreads ];
    };
  };
}).overrideAttrs (attrs:
  {
    package = attrs.windows;
    rust = attrs.windows;
  }
)

