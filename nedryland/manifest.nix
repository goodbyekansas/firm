{ pkgs, name, manifest, code, attachments }:
let
  genAttachmentAttrs = attachmentPathOrAttrs: attachmentName: idx:
    let
      checksums = { checksums = { sha256 = "@sha256_${builtins.toString idx}@"; }; };
      path = { path = "@attachment_${builtins.toString idx}@"; };
      metadata = (
        if builtins.isString attachmentPathOrAttrs || builtins.isPath attachmentPathOrAttrs then {
          metadata = { };
        } else {
          metadata = attachmentPathOrAttrs.metadata or { };
        }
      );
    in
    {
      "${attachmentName}" = ({ } // metadata // checksums // path);
    };

  getAttachmentPath = attachmentPathOrAttrs:
    (
      if builtins.isString attachmentPathOrAttrs || builtins.isPath attachmentPathOrAttrs then
        attachmentPathOrAttrs
      else
        attachmentPathOrAttrs.path
    );
  manifestContent = if builtins.isPath manifest then (builtins.fromTOML (builtins.readFile manifest)) else manifest;
  attachmentList = pkgs.lib.mapAttrsToList
    (key: value: {
      attachmentName = "${key}";
      data = value;
    })
    attachments;

  nestedAttachmentList = pkgs.lib.imap1 (i: att: genAttachmentAttrs att.data att.attachmentName i) attachmentList;
  flattenedAttachmentList = (pkgs.lib.lists.fold (a: b: a // b) { } nestedAttachmentList);
  manifestWithChecksum = manifestContent // {
    attachments = flattenedAttachmentList;
  }
    // genAttachmentAttrs code "code" 0;
  # Code path is treated differently because it's relative to the installation folder rather than the component
  # and attachment paths are always relative to the component or are an absolute path
  # Could be possible to unify code with attachments
  codePath = "${getAttachmentPath code}";
  attachmentPaths = pkgs.lib.lists.forEach attachmentList (att: (getAttachmentPath att.data));
  attachmentNames = pkgs.lib.lists.forEach attachmentList (att: (att.attachmentName));
in
pkgs.stdenv.mkDerivation {
  name = "${name}-manifest";
  nativeBuildInputs = [ pkgs.utillinux pkgs.remarshal ];

  inherit attachmentPaths codePath attachmentNames;
  manifestContent = builtins.toJSON manifestWithChecksum;
  passAsFile = [ "manifestContent" ];

  phases = [ "buildPhase" "installPhase" "fixupPhase" ];

  buildPhase = ''
    json2toml -i $manifestContentPath -o ./manifest.toml
  '';

  installPhase = ''
    mkdir -p $out
    cp manifest.toml $out/manifest.toml
  '';

  setupHook = ./generate-manifest.sh;
}
