{ writeScriptBin
, openssh
, wineWowPackages
, bash
, makeSetupHook
, writeTextFile
, system
}:
makeSetupHook
{
  name = "windows-runner-setup-hook";
  substitutions = {
    runner =
      writeScriptBin "windows-runner" ''
        #! ${bash}/bin/bash
        set -e

        if [ -n "$NEDRYGLOT_WINDOWS_HOST" ]; then
            hostname="localhost"
            username="User"
            IFS='@' read user host <<< "$NEDRYGLOT_WINDOWS_HOST"
            if [ -n "$host" ]; then
              hostname="$host"
              username="$user"
            else
              hostname="$user"
            fi

            port=""
            IFS=':' read host p <<< "$hostname"
            if [ -n "$p" ]; then
              hostname="$host"
              port="$p"
            fi

            if [ -n "$port" ]; then
               sshFlags+=("-p $port")
            fi
            sshFlags=("-o StrictHostKeyChecking=no" "-o BatchMode=yes" "-o UserKnownHostsFile=/dev/null" "-F ${openssh}/etc/ssh/ssh_config")
            echo -n "Waiting for ssh to come up..."
            until $(${openssh}/bin/ssh -q -o ConnectTimeout=1 ''${sshFlags[*]} "$username"@"$hostname" "if not exist "rust" mkdir rust && exit"); do
              echo -n "."
              sleep 4
            done

            executable=$(basename "$1")
            echo "ssh is up, running!"
            ${openssh}/bin/scp ''${sshFlags[*]/-p/-P} "$1" "$username"@"$hostname":rust/$executable
            shift
            ${openssh}/bin/ssh -t ''${sshFlags[*]} "$username"@"$hostname" ".\\rust\\$executable" "$@"
        elif [ -n "$WSL_DISTRO_NAME" ]; then
          "$@"
        ${if system == "x86_64-linux" then ''
        elif [ -z "$NEDRYGLOT_DONT_USE_WINE" ]; then
            echo "🍷 NEDRYGLOT_WINDOWS_HOST not set, trying wine..."
            cacheFolder="''${XDG_CACHE_HOME:-$HOME/.cache}"/nedryglot/
            if [ ! -w "$HOME" ]; then
              cacheFolder=$(mktemp -d --tmpdir nedryglot-wine-XXXXX)
            fi
            mkdir -p "$cacheFolder/.wine"
            cacheFolder="$cacheFolder"
            export WINEPREFIX="$cacheFolder"/.wine
            export WINEDEBUG=fixme-all,warn-all
            export WINEDLLOVERRIDES='mscoree,mshtml='
            echo "🍾 Starting wineserver..."
            ${wineWowPackages.stable}/bin/wineserver --persistent=300 || echo "🥂 Wineserver already running"
            ${wineWowPackages.stable}/bin/wine64 "$@"
          ''
        else ""}
        else
          echo "Please set NEDRYGLOT_WINDOWS_HOST to a Windows host where you have SSH access."
          exit 1
        fi
      '';

    postShell = writeTextFile {
      name = "post-shell";
      executable = true;
      text = ''
        nedryglotVmInfo() {
          echo "🏘 Set NEDRYGLOT_WINDOWS_HOST to a hostname of a Windows machine"
          echo "  where you have SSH access (without password)."
          echo "  cargo run/test will use this host to run/test the code."
          echo "  If you need a VM to do this you may use https://developer.microsoft.com/en-us/windows/downloads/virtual-machines/"
        }
      '';
    };
  };
}
  (builtins.toFile "windows-runner-hook" ''
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=@runner@/bin/windows-runner
    postShell="''${postShell:-} source @postShell@; nedryglotVmInfo"
  '')
