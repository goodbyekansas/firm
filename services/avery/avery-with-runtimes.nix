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

      cp -r --no-preserve=mode $avery/. $out
      chmod +x $out/bin/avery

      for runtime in $runtimes; do
        cp -r $runtime/. $out
      done

      mkdir -p $out/etc/avery
      echo "{\"runtime_directories\": [\"$out/share/avery/runtimes\"]}" >$out/etc/avery/avery.json
      wrapProgram $out/bin/avery --set AVERY_CONFIG $out/etc/avery/avery.json;
    '';
  };
}
