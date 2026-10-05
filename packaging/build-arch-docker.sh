#!/usr/bin/env bash
# Build the Arch package inside a clean archlinux container (same as CI). Useful when the host
# toolchain is in a partial-upgrade state. Output: target/pkg/*.pkg.tar.zst
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
ver=$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/Cargo.toml" | head -1)
out="$here/target/pkg"
mkdir -p "$out"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git -C "$here" ls-files -z | tar -C "$here" --null -T - --transform "s|^|sordino-$ver/|" -czf "$work/src.tar.gz"
cp "$here/packaging/aur/PKGBUILD" "$work/"
docker run --rm -v "$work:/pkg" -v "$out:/out" archlinux:latest bash -c '
  set -e
  pacman -Syu --noconfirm --needed base-devel git rust clang nodejs npm pipewire wireplumber \
    webkit2gtk-4.1 gtk3 libayatana-appindicator webrtc-audio-processing xdg-utils >/dev/null
  useradd -m builder
  chown -R builder /pkg
  cd /pkg
  su builder -c "SORDINO_SOURCE_URL=file:///pkg/src.tar.gz makepkg --nocheck --noconfirm"
  cp /pkg/*.pkg.tar.zst /out/
'
ls -la "$out"/*.pkg.tar.zst
