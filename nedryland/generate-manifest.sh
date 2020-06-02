fixupOutputHooks+=('generateManifest')

generateManifest() {
  codepath=(@codePath@)
  attachmentPaths=(@attachmentPaths@)
  attachmentNames=(@attachmentNames@)
  attachmentTest=(@attachmentTest@)

    echo "📜 Creating output manifest..."

  # relative paths are relative to the derivation
  # output folder (or really, the manifest file but that
  # is the same).

  function makeSha256() {
    att=$(realpath "$1")
    if [ ! -f $att ]; then
      echo "ERROR: 💥 specified attachment \"$att\" does not exist..."
      exit 1
    fi
    echo "generating checksum for $att..."
    checksum=$(sha256sum $att | cut -d " " -f 1)
  }

  # Code path is relative to the installation directory ($out in this case)
  # while the rest are relative to the component
  pushd $out > /dev/null
  makeSha256 $codepath
  declare -x "sha256_0=$checksum"
  declare -x "attachment_0=$codepath"
  popd > /dev/null

  mkdir "$out/attachments"
  for i in "${!attachmentPaths[@]}"; do
    makeSha256 "${attachmentPaths[$i]}"
    declare -x "sha256_$(($i+1))=$checksum"

    # copy files
    att=$(realpath "${attachmentPaths[$i]}")
    extension="$(sed 's/^\w\+.//' <<< $(basename $att))"
    
    if [ -z "$extension" ];
    then
        targetFileName="${attachmentNames[$i]}"
    else
        targetFileName="${attachmentNames[$i]}.$extension"
    fi

    targetpath="$out/attachments/$targetFileName"
    echo "copying $att to installation $targetpath"

    cp $att $targetpath

    # declare replacable for the file name in the manifest
    declare -x "attachment_$(($i+1))=attachments/$targetFileName"

  done
  substituteAll @out@/manifest.toml $out/manifest.toml

  echo "📜 Manifest written to $out/manifest.toml"
}
