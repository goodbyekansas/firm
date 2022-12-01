{ pkgs, python3, name, manifest, bash, j2cli }:
let
  normalizeAttachments = pkgs.lib.mapAttrs
    (
      _: path:
        if builtins.isString path || builtins.isPath path then {
          inherit path;
        }
        else path
    );

  manifestWithNormalizedAttachments = {
    # wrap this in a key to be able to
    # use variables with dashes in Jinja later
    manifest = manifest // {
      attachments = normalizeAttachments manifest.attachments or { };
    };
  };
in
pkgs.stdenv.mkDerivation {
  inherit bash j2cli python3;
  name = "${name}-manifest";

  manifestData = builtins.toJSON manifestWithNormalizedAttachments;
  passAsFile = [ "manifestData" ];

  # we need the fixupPhase to exist even though
  # we do not define one but otherwise the `setupHook`
  # below does not get written out (it happens during stdenv's
  # fixupPhase)
  phases = [ "installPhase" "fixupPhase" ];

  installPhase = ''
    mkdir -p $out
    cp ${./generate_checksums.py} $out/generate_checksums.py
    cp ${./copy_attachments.py} $out/copy_attachments.py
    cp ${./manifest-template.jinja.toml} $out/manifest-template.jinja.toml
    cp ${./template_settings.py} $out/template_settings.py
    cp $manifestDataPath $out/manifest-data.json
  '';

  setupHook = ./generate-manifest.sh;
}
