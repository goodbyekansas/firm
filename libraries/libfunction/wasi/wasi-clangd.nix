{ stdenv, writeShellScriptBin }:
writeShellScriptBin "clangd" ''
  buildcpath() {
    local path after
    while (( $# )); do
      case $1 in
          -isystem)
              shift
              path=$path''${path:+':'}$1
              ;;
          -idirafter)
              shift
              after=$after''${after:+':'}$1
              ;;
      esac
      shift
    done
    echo $path''${after:+':'}$after
  }

  export CPATH=''${CPATH}''${CPATH:+':'}$(buildcpath ''${NIX_CFLAGS_COMPILE} \
                                        $(<${stdenv.cc}/nix-support/libc-cflags)):${stdenv.cc}/resource-root/include

  export CPLUS_INCLUDE_PATH=''${CPLUS_INCLUDE_PATH}''${CPLUS_INCLUDE_PATH:+':'}$(buildcpath ''${NIX_CFLAGS_COMPILE} \
                                                                               $(<${stdenv.cc}/nix-support/libcxx-cxxflags) \
                                                                               $(<${stdenv.cc}/nix-support/libc-cflags)):${stdenv.cc}/resource-root/include

  exec -a "$0" ${stdenv.cc.cc}/bin/$(basename $0) "$@"
''

