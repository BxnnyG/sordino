#!/usr/bin/env bash
# Create the release signing key for in-app updates. Run it yourself, once:
#
#   tools/make-signing-key.sh
#
# * Generates a minisign key pair in a throw-away container (nothing is installed on the host).
# * The SECRET key goes straight into the GitHub repository secret MINISIGN_SECRET_KEY (via the
#   gh CLI you are logged in with) and is then wiped. It never touches the repository.
# * The PUBLIC key is written to packaging/minisign.pub; commit that file.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
pub="$here/packaging/minisign.pub"
repo=BxnnyG/sordino

if grep -q '^RW' "$pub" 2>/dev/null && [ "${1:-}" != "--replace" ]; then
  echo "A signing key already exists ($pub). Use --replace to make a new one" >&2
  echo "(older app versions will then refuse updates until they are updated by hand)." >&2
  exit 1
fi
command -v docker >/dev/null || { echo "needs docker" >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "log in with 'gh auth login' first" >&2; exit 1; }

# A RAM-backed private directory, removed whatever happens.
dir=$(mktemp -d "${XDG_RUNTIME_DIR:-/tmp}/sordino-key.XXXXXX")
chmod 700 "$dir"
trap 'shred -u "$dir"/* 2>/dev/null || true; rm -rf "$dir"' EXIT

docker run --rm -v "$dir:/k" debian:stable-slim sh -c "
  apt-get update -qq && apt-get install -y -qq minisign >/dev/null &&
  minisign -G -W -p /k/minisign.pub -s /k/minisign.key >/dev/null &&
  chown -R $(id -u):$(id -g) /k"

gh secret set MINISIGN_SECRET_KEY --repo "$repo" < "$dir/minisign.key"
cp "$dir/minisign.pub" "$pub"
echo
echo "Done. The secret key is stored as the GitHub secret MINISIGN_SECRET_KEY and was wiped here."
echo "Public key written to packaging/minisign.pub:"
cat "$pub"
