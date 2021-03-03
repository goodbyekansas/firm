{ stdenv, base, runtimes, avery, makeWrapper }:
let
  flattenAttrs = attrs: builtins.map (value: value.package) (builtins.attrValues attrs);
  flattenedRuntimes = flattenAttrs runtimes;
in
{ additionalRuntimes ? { } }:
let
  flattenedAdditionalRuntimes =
    if builtins.isList additionalRuntimes then
      additionalRuntimes
    else
      flattenAttrs additionalRuntimes;
in
base.mkComponent {
  name = "avery-bundle";
  package = stdenv.mkDerivation {
    name = "avery-bundle";
    runtimes = flattenedRuntimes ++ flattenedAdditionalRuntimes;
    avery = avery.package;
    nativeBuildInputs = [ makeWrapper ];

    builder = builtins.toFile "builder.sh" ''
      source $stdenv/setup
      mkdir -p $out

      cp -r --no-preserve=mode,ownership,timestamps $avery/. $out
      chmod +x $out/bin/avery

      # Move all runtimes. This will overwrite the .checksums.toml
      for runtime in $runtimes; do
        cp -r --no-preserve=mode,ownership,timestamps $runtime/. $out
      done

      # Create our concatenated .checksums.toml
      rm $out/share/avery/runtimes/.checksums.toml

      for runtime in $runtimes; do
        cat $runtime/share/avery/runtimes/.checksums.toml >> $out/share/avery/runtimes/.checksums.toml
      done

      mkdir -p $out/etc/avery
      echo "{\"runtime_directories\": [\"$out/share/avery/runtimes\"]}" >$out/etc/avery/avery.json
      wrapProgram $out/bin/avery --set AVERY_CONFIG $out/etc/avery/avery.json;
    '';
  };
}
