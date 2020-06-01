{ pkgs, name, manifest, code, attachments }:
let
  genAttachmentAttrs = attachmentPathOrAttrs: name: idx:
    let
      checksums = {sha256 = "@sha256_${builtins.toString idx}@";};
      content = 
    (if builtins.isString attachmentPathOrAttrs || builtins.isPath attachmentPathOrAttrs then
        {
          path = attachmentPathOrAttrs;
          metadata = { };
        }
      else
        {
          path = attachmentPathOrAttrs.path;
          metadata = attachmentPathOrAttrs.metadata or { };
        }
    );
    in
    {
      "${name}"= content // checksums;
    };

  manifestContent = if builtins.isPath manifest then (builtins.fromTOML (builtins.readFile manifest)) else manifest;
  attachmentList = pkgs.lib.mapAttrsToList (key: value: {"${key}"= value;}) attachments;
  manifestWithChecksum = manifestContent // {
    attachments = pkgs.lib.imap1 (i: att: genAttachmentAttrs att i) attachmentList;
  }
   // cgenAttachmentAttrs code "code" 0;

  attachmentPaths = [ manifestWithChecksum.code.path ] ++ (builtins.map (att: att.path) manifestWithChecksum.attachments);

in
pkgs.stdenv.mkDerivation
{
  name = "${name}-manifest";
  nativeBuildInputs = [ pkgs.utillinux pkgs.remarshal ];

  inherit attachmentPaths;
  manifestContent = builtins.toJSON manifestWithChecksum;
  passAsFile = [ "manifestContent" ];

  phases = [ "buildPhase" "installPhase" "fixupPhase" ];

  buildPhase = ''
      #source $stdenv/setup
      json2toml -i $manifestContentPath -o ./manifest.toml
      cat ./manifest.toml
  '';

  installPhase = ''
      mkdir -p $out
      cp manifest.toml $out/manifest.toml
  '';

  setupHook = ./generate-manifest.sh;
}
