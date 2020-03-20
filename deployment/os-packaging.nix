{ pkgs, base, linuxPackages, windowsPackages }:
with pkgs;
base.mkComponent {
  package = stdenv.mkDerivation {
    name = "os-packages";

    linuxPackages = builtins.map (p: p.package) linuxPackages;
    windowsPackages = builtins.map (p: p.package) windowsPackages;

    # TODO: we should be better at not inlining shell
    # scripts 👼 
    builder = builtins.toFile "builder.sh" ''
      source $stdenv/setup
      mkdir -p $out/linux
      mkdir -p $out/windows
      echo "this is the linux packages"
      echo $linuxPackages
      for lp in $linuxPackages; do
         cp -r $lp/bin $out/linux
      done

      echo "This is temp output and will not be printed here"
      echo "📦 building RPM package..."

      echo "done!"

      echo "🏁 building windows installer..."

      echo "done!"
    '';

  };

  deployment = {
    windowsInstaller = base.mkWindowsInstaller {};
    rpmPackage = base.mkRPMPackage {};

    upload = base.uploadFiles [
      windowsInstaller
      rpmPackage
    ];
  };
}
