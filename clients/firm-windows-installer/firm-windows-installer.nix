{ base
, symlinkJoin
, writeScript
, stdenv
, lib
, version
, avery
, bendini
, lomax
, configFiles ? null
, additionalRuntimes ? { }
, windowsInstall
}:
let
  avery' = (avery.withRuntimes.override { generateConfig = false; }) { inherit additionalRuntimes; };
  bundle = symlinkJoin {
    name = "firm-install-bundle";
    paths = [ avery'.windows lomax.windows bendini.windows ]
      ++ (lib.optional (configFiles != null) configFiles);
  };
  archive = stdenv.mkDerivation {
    name = "firm-install-archive.tar.gz";
    builder = writeScript "builder.sh" ''
      source $stdenv/setup
      tar chzf $out -C ${bundle} --mode='a+rwX' .
    '';
  };
in
(base.languages.rust.nativeTools.override {
  crossTargets.rust = base.languages.rust.nativeTools.crossTargets.windows;
}).mkClient rec {
  name = "firm-windows-installer";
  src = ./.;
  inherit version;
  buildInputs = [ windowsInstall ];

  shellHook = ''
    copyInstaller() {
      cargo build

      # installer
      tempWslInstaller=$(wslpath "$(wslvar USERPROFILE)"/AppData/Local/Temp/firm-installer.exe)
      cp --no-preserve=mode target/x86_64-pc-windows-gnu/debug/firm-installer.exe $tempWslInstaller
    }

    ln -fs ${archive} install-data
    echo -e "🧪 To test the windows installer use \033[95mcopyInstaller\033[0m to copy firm-installer.exe to a windows temp folder"
    echo "   then run it in an elevated prompt"
  '';
  preBuildPhases = [ "linkDataPhase" ];
  linkDataPhase = ''
    ln -fs ${archive} install-data
  '';
}
