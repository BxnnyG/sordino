#!/usr/bin/env bash
# Run cargo (or any command) inside the dev container, with persistent registry and target caches.
#   tools/dcargo.sh cargo test --profile fast -p sordino-core
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
docker image inspect sordino-dev >/dev/null 2>&1 || docker build -t sordino-dev -f "$here/tools/Dockerfile.dev" "$here/tools"
# The cache volumes must belong to the calling user (the container runs as that user so that
# files written into the checkout are not owned by root).
if [ "$(docker run --rm -v sordino-cargo:/cargo sordino-dev stat -c %u /cargo)" != "$(id -u)" ]; then
  docker run --rm -v sordino-cargo:/cargo -v sordino-target:/target sordino-dev chown -R "$(id -u):$(id -g)" /cargo /target
fi
exec docker run --rm -t --user "$(id -u):$(id -g)" -e HOME=/tmp -v "$here:/w" -v sordino-cargo:/cargo -v sordino-target:/target -w /w sordino-dev "$@"
