#!/usr/bin/env bash
# Проверка переносимости: cargo check портируемых крейтов под платформенные цели.
#
# Использование:
#   ./check-targets.sh                  # Linux (нативный) + Apple + Android
#   ./check-targets.sh <target> [...]   # только указанные цели
#
# Скрипт только проверяет компиляцию (без линковки и без SDK), поэтому Apple/Android-цели
# работают и на Linux-машине. Настоящие сборки требуют Xcode (macOS/iOS) и NDK (Android).
set -u
here=$(cd "$(dirname "$0")" && pwd)
export CARGO_HOME=${CARGO_HOME:-$here/.cargo}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$here/target}
# Локальный RUSTUP_HOME: домашний каталог rustup в песочнице только для чтения,
# поэтому цели ставятся в рабочую копию (см. PORTING.md → «Сборка под платформы»).
if [ -d "$here/.rustup/toolchains" ]; then
  export RUSTUP_HOME=${RUSTUP_HOME:-$here/.rustup}
fi

TARGETS=("$@")
if [ ${#TARGETS[@]} -eq 0 ]; then
  TARGETS=(x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu \
           x86_64-pc-windows-gnu x86_64-pc-windows-msvc \
           aarch64-apple-darwin aarch64-apple-ios aarch64-linux-android)
fi

# Крейты без платформенной специфики: ядро, данные, скрипты, CLI.
CORE=(vmix-xml vmix-config vmix-functions vmix-api vmix-providers vmix-script vmixctl)
# Крейты с нативными зависимостями: их проверяем отдельно и отмечаем ожидаемые ограничения.
NATIVE=(vmix-ndi vmix-midi vmix-streamdeck vmixui)

command -v rustup >/dev/null || { echo "нужен rustup"; exit 1; }

for target in "${TARGETS[@]}"; do
  if ! rustup target list --installed | grep -qx "$target"; then
    echo "== добавляю цель $target"
    rustup target add "$target" >/dev/null 2>&1 || { echo "   не удалось добавить"; continue; }
  fi
done

status() { # status <target> <crate...>
  local target=$1; shift
  local line=""
  for crate in "$@"; do
    if cargo check -q -p "$crate" --target "$target" >/tmp/check.log 2>&1; then
      line+="$(printf '%-14s ok   ' "$crate")"
    else
      local reason
      reason=$(grep -m1 -E '^error' /tmp/check.log | cut -c1-60)
      line+="$(printf '%-14s FAIL ' "$crate")"
      echo "   $crate → $reason" >&2
    fi
  done
  echo "$target"
  echo "  $line"
}

for target in "${TARGETS[@]}"; do
  echo "== $target"
  status "$target" "${CORE[@]}"
done

echo
echo "== крейты с нативными зависимостями (ожидаются ограничения)"
for target in "${TARGETS[@]}"; do
  echo "-- $target"
  status "$target" "${NATIVE[@]}"
done
