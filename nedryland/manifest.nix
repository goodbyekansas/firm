{ pkgs, name, manifest }:
let
  manifestData = (if builtins.isPath manifest then (builtins.fromTOML (builtins.readFile manifest)) else manifest);
  j2 = pkgs.python3.withPackages (ps: with ps; [ j2cli setuptools ]);

  normalizeAttachments = attachments: pkgs.lib.mapAttrs
    (
      attName: attachmentPathOrData:
        if builtins.isString attachmentPathOrData || builtins.isPath attachmentPathOrData then {
          path = attachmentPathOrData;
        }
        else attachmentPathOrData
    )
    attachments;

  manifestWithNormalizedAttachments = {
    # wrap this in a key to be able to
    # use variables with dashes in Jinja later
    manifest = manifest // {
      attachments = normalizeAttachments manifest.attachments or { };
    };
  };
in
pkgs.stdenv.mkDerivation {
  name = "${name}-manifest";
  propagatedBuildInputs = [ j2 ];
  manifestData = builtins.toJSON manifestWithNormalizedAttachments;
  passAsFile = [ "manifestData" ];

  # we need the fixupPhase to exist even though
  # we do not define one but otherwise the `setupHook`
  # below does not get written out (it happens during stdenv's
  # fixupPhase
  phases = [ "installPhase" "fixupPhase" ];

  installPhase = ''
    mkdir -p $out
    cp ${./generate_checksums.py} $out/generate_checksums.py
    cp ${./copy_attachments.py} $out/copy_attachments.py
    cp ${./manifest-template.jinja.toml} $out/manifest-template.jinja.toml
    cp $manifestDataPath $out/manifest-data.json
  '';

  setupHook = ./generate-manifest.sh;
}
