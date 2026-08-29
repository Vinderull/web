#!/usr/bin/env bash
# Regenerate static/js/htmx.min.js from a pinned htmx release.
#
# The runtime image is scratch with CSP `default-src 'self'`, so htmx is
# committed into the repo and baked into the image by COPY; there is no CDN.
# This script is the upgrade path: it downloads the pinned release, verifies
# it against the pinned SHA-256 (aborting on any mismatch), and installs it.
#
# Upgrade procedure:
#   1. Set VERSION to the target release.
#   2. Set SHA256 to that release's dist/htmx.min.js hash. unpkg publishes it
#      as the `integrity` field of /htmx.org@<version>/?meta (base64 sha256;
#      decode with `base64 -d | xxd -p`). Never bump a version without
#      updating the hash — the script refuses mismatches.
#   3. Run `sh scripts/update-htmx.sh`; it prints the SRI sha384 value to put
#      on the <script> tag in templates/base.html.
#   4. Commit the new file, the new integrity attribute, and the new hash
#      together.
#
# `--check` (used by CI) verifies the committed file is byte-identical to the
# pinned release without writing anything, catching hand edits.
set -euo pipefail

VERSION="4.0.0"
SHA256="e484d9171a9db30a39c8f16e3d709d4137f3211c659f8e6125816635033d593f"
URL="https://unpkg.com/htmx.org@${VERSION}/dist/htmx.min.js"
OUT="static/js/htmx.min.js"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

curl -fsSL "$URL" -o "$tmp"
echo "$SHA256  $tmp" | sha256sum -c - >/dev/null   # abort on silent upstream change

case "${1:-}" in
  --check)
    if cmp -s "$tmp" "$OUT"; then
      echo "ok: $OUT matches htmx ${VERSION}"
    else
      echo "error: $OUT differs from pinned htmx ${VERSION}" >&2
      echo "       run: sh scripts/update-htmx.sh  and commit the result" >&2
      exit 1
    fi
    ;;
  "")
    install -m 0644 "$tmp" "$OUT"
    echo "vendored htmx ${VERSION} (${SHA256:0:12})"
    echo "SRI integrity for templates/base.html:"
    printf '  integrity="sha384-%s"\n' "$(openssl dgst -sha384 -binary "$OUT" | openssl base64 -A)"
    ;;
  *)
    echo "usage: $0 [--check]" >&2
    exit 2
    ;;
esac
