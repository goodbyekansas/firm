{ stdenv, base, runtimes, avery, makeWrapper, generateConfig ? true }:
let
  flattenAttrs = attrs: builtins.map (value: value.wasi) (builtins.attrValues attrs);
  flattenedRuntimes = flattenAttrs runtimes;
in
{ additionalRuntimes ? { } }:
let
  flattenedAdditionalRuntimes =
    if builtins.isList additionalRuntimes then
      additionalRuntimes
    else
      flattenAttrs additionalRuntimes;
  mkBundle = iswindows:
    let
      avery' = if iswindows then avery.windows else avery.package;
      runtimeRoot = if iswindows then "avery" else "share/avery";
      configRoot = if iswindows then "" else "etc/avery";
      name = if iswindows then "avery-bundle-windows" else "avery-bundle";
      exe = if iswindows then "avery.exe" else "avery";
    in
    stdenv.mkDerivation {
      inherit name;
      avery = avery';
      runtimes = flattenedRuntimes ++ flattenedAdditionalRuntimes;
      nativeBuildInputs = [ makeWrapper ];

      builder = builtins.toFile "builder.sh" ''
        source $stdenv/setup
        mkdir -p $out/${runtimeRoot}/runtimes/

        cp -r --no-preserve=mode,ownership,timestamps $avery/. $out
        chmod +x $out/bin/${exe}

        # Move all runtimes. This will overwrite the .checksums.toml
        # so we append to a file in working dir and move it later
        shopt -s dotglob
        for runtime in $runtimes; do
          cp -r --no-preserve=mode,ownership,timestamps $runtime/share/avery/runtimes/. $out/${runtimeRoot}/runtimes
          cat $runtime/share/avery/runtimes/.checksums.toml >> .checksums.toml
        done
      
        mv .checksums.toml $out/${runtimeRoot}/runtimes/
        shopt -u dotglob
      
        ${if generateConfig then ''
          mkdir -p $out/${configRoot}
          echo "{\"runtime_directories\": [\"$out/${runtimeRoot}/runtimes\"]}" >$out/${configRoot}/avery.json
          wrapProgram $out/bin/${exe} --set AVERY_CONFIG $out/${configRoot}/avery.json;
        '' else ""}
      '';
    };
in
(base.mkComponent {
  name = "avery-bundle";
  package = mkBundle false;
  windows = mkBundle true;
})
