#! @bash@
# shellcheck disable=SC2239
# shellcheck shell=bash

copyAttachments() {
  echo "  📜 [manifest] Copying attachments to output folder..."
  # shellcheck disable=SC2154
  mkdir -p "$out/attachments"
  python @out@/copy_attachments.py \
    "$out/attachments" \
    @out@/manifest-data.json \
    "$NIX_BUILD_TOP/manifest-data-with-existing-attachments.json" | sed "s/^/  📜 [manifest] /"
}

generateManifestPhase() {

  echo "  📜 [manifest] Creating output manifest..."

  echo "  📜 [manifest] Generating checksums for attachments..."
  python @out@/generate_checksums.py \
    "$NIX_BUILD_TOP/manifest-data-with-existing-attachments.json" \
    "$NIX_BUILD_TOP/manifest-data-with-checksums.json" \
    --sha512 | sed "s/^/  📜 [manifest] /"

  echo "  📜 [manifest] Generating final manifest..."
  j2 -f json --customize @out@/template_settings.py \
    @out@/manifest-template.jinja.toml \
    "$NIX_BUILD_TOP/manifest-data-with-checksums.json" \
    -o "$out/manifest.toml" | sed "s/^/  📜 [manifest] /"
  echo "  📜 [manifest] Manifest written to $out/manifest.toml"
}

postInstallHooks+=(copyAttachments)
preDistPhases+=" generateManifestPhase"

