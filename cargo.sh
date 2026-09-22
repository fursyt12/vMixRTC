#!/usr/bin/env bash
# Cargo с CARGO_HOME внутри workspace.
# Причина: ~/.cargo в этом окружении доступен только на чтение, а cargo пишет
# туда индекс и кэш. Поэтому реестр держим рядом с проектом.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export CARGO_HOME="${CARGO_HOME:-$here/.cargo}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$here/target}"

exec cargo "$@"
