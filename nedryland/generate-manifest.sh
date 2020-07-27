preDistPhases+=" generateManifestPhase"

generateManifestPhase() {

  echo "  📜 [manifest] Creating output manifest..."

  echo "  📜 [manifest] Generating checksums for attachments..."
  python @out@/generate_checksums.py \
          @out@/manifest-data.json \
          ./manifest-data-with-checksums.json \
          --sha512 | sed "s/^/  📜 [manifest] /"

  echo "  📜 [manifest] Copying attachments to output folder..."
  mkdir -p $out/attachments
  python @out@/copy_attachments.py \
          $out/attachments \
          ./manifest-data-with-checksums.json \
          ./manifest-data-with-existing-attachments.json | sed "s/^/  📜 [manifest] /"

  echo "  📜 [manifest] Generating final manifest..."
  j2 -f json @out@/manifest-template.jinja.toml \
          ./manifest-data-with-existing-attachments.json \
          -o $out/manifest.toml | sed "s/^/  📜 [manifest] /"
  echo "  📜 [manifest] Manifest written to $out/manifest.toml"
}
