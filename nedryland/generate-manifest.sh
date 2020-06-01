fixupOutputHooks+=('generateManifest')

generateManifest() {
  attachmentPaths=@attachmentPaths@

  echo "📜 Creating output manifest..."

  # relative paths are relative to the derivation
  # output folder (or really, the manifest file but that
  # is the same).
  pushd $out > /dev/null
  for i in "${!attachmentPaths[@]}"; do
    att=$(realpath "${attachmentPaths[$i]}")
    if [ ! -f $att ]; then
      echo "ERROR: 💥 specified attachment \"$att\" does not exist..."
      exit 1
    fi

    echo "generating checksum for attachment at $att..."

    declare -x "sha256_$i=$(sha256sum $att | cut -d " " -f 1)"

  done
  substituteAll @out@/manifest.toml $out/manifest.toml
  popd > /dev/null

  echo "📜 Manifest written to $out/manifest.toml"
}
