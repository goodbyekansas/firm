generateManifest() {

  echo "  📜 [manifest] Creating output manifest..."

  echo "  📜 [manifest] Copying attachments to output folder..."
  mkdir -p $out/attachments
  python @out@/copy_attachments.py \
    $out/attachments \
    @out@/manifest-data.json \
    ./manifest-data-with-existing-attachments.json | sed "s/^/  📜 [manifest] /"

  echo "  📜 [manifest] Generating checksums for attachments..."
  python @out@/generate_checksums.py \
    ./manifest-data-with-existing-attachments.json \
    ./manifest-data-with-checksums.json \
    --sha512 | sed "s/^/  📜 [manifest] /"

  echo "  📜 [manifest] Generating final manifest..."
  j2 -f json --customize @out@/template_settings.py \
    @out@/manifest-template.jinja.toml \
    ./manifest-data-with-checksums.json \
    -o $out/manifest.toml | sed "s/^/  📜 [manifest] /"
  echo "  📜 [manifest] Manifest written to $out/manifest.toml"
}

# Do not actually add generateManifest to any phases. We will insert an
# explicit call to it in the installPhase of the package.
