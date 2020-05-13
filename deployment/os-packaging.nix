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
      mkdir -p $out/linux/bin
      mkdir -p $out/windows/bin
      echo "this is the linux packages"
      echo $linuxPackages
      for lp in $linuxPackages; do
         cp -r $lp/bin/* $out/linux/bin
      done

      echo "This is temp output and will not be printed here"
      echo "📦 building RPM package..."

      echo "done!"

      echo "🏁 building windows installer..."

      echo "done!"
    '';

  };

  deployment = {
    windowsInstaller = base.deployment.mkWindowsInstaller { };
    rpmPackage = base.deployment.mkRPMPackage { };

    upload = base.deployment.uploadFiles [
      windowsInstaller
      rpmPackage
    ];
  };
}
