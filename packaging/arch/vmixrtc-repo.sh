#!/usr/bin/env bash
#
# Добавляет pacman-репозиторий vMixRTC в систему.
#
# В Arch нет аналога add-apt-repository: сторонние репозитории подключаются правкой
# /etc/pacman.conf. Этот скрипт делает то же самое одной командой и идемпотентно:
# кладёт отдельный конфиг репозитория в /etc/pacman.d/ и добавляет на него Include,
# сохраняя резервную копию pacman.conf.
#
# Установка:
#   curl -fsSLO https://github.com/fursyt12/vMixRTC/releases/download/repo-x86_64/vmixrtc-repo.sh
#   sudo bash vmixrtc-repo.sh
#   sudo pacman -S vmixrtc-bin
#
# Удаление:
#   sudo bash vmixrtc-repo.sh --remove
#
set -euo pipefail

REPO_NAME="vmixrtc"
REPO_URL="${VMIXRTC_REPO_URL:-https://github.com/fursyt12/vMixRTC/releases/download/repo-x86_64}"
CONF_DIR="${VMIXRTC_CONF_DIR:-/etc/pacman.d}"
PACMAN_CONF="${VMIXRTC_PACMAN_CONF:-/etc/pacman.conf}"
REPO_CONF="$CONF_DIR/$REPO_NAME.conf"
INCLUDE_LINE="Include = $REPO_CONF"

say() { printf '\033[1m%s\033[0m\n' "$*"; }
die() { printf 'ошибка: %s\n' "$*" >&2; exit 1; }

case "${1:-}" in
  -h|--help)
    cat <<EOF
Использование: $0 [--remove] [--no-sync]

  (без аргументов)  добавить репозиторий и обновить базы (pacman -Sy)
  --remove          убрать репозиторий и обновить базы
  --no-sync         ничего не синхронизировать (только правка конфигов)

Переменные для тестов: VMIXRTC_CONF_DIR, VMIXRTC_PACMAN_CONF, VMIXRTC_REPO_URL,
VMIXRTC_ALLOW_NON_ROOT=1.
EOF
    exit 0
    ;;
esac

REMOVE=0
SYNC=1
for arg in "$@"; do
  case "$arg" in
    --remove) REMOVE=1 ;;
    --no-sync) SYNC=0 ;;
    *) die "неизвестный аргумент: $arg (см. --help)" ;;
  esac
done

if [[ "$(id -u)" -ne 0 && "${VMIXRTC_ALLOW_NON_ROOT:-0}" != "1" ]]; then
  die "нужны права root: sudo bash $0"
fi
command -v pacman >/dev/null || die "pacman не найден — это скрипт для Arch Linux"

backup() {
  local stamp
  stamp="$(date +%Y%m%d-%H%M%S)"
  cp -a "$PACMAN_CONF" "$PACMAN_CONF.vmixrtc-$stamp.bak"
  echo "  резервная копия: $PACMAN_CONF.vmixrtc-$stamp.bak"
}

if [[ "$REMOVE" -eq 1 ]]; then
  say "Удаляю репозиторий $REPO_NAME"
  [[ -f "$PACMAN_CONF" ]] || die "$PACMAN_CONF не найден"
  if grep -qF "$INCLUDE_LINE" "$PACMAN_CONF"; then
    backup
    # убираем строку Include и оставленный комментарий-заголовок
    grep -vF -e "$INCLUDE_LINE" -e "# vMixRTC repository" "$PACMAN_CONF" > "$PACMAN_CONF.tmp"
    mv "$PACMAN_CONF.tmp" "$PACMAN_CONF"
    echo "  строка Include удалена"
  else
    echo "  строка Include не найдена — нечего править"
  fi
  [[ -f "$REPO_CONF" ]] && rm -f "$REPO_CONF" && echo "  удалён $REPO_CONF"
  [[ "$SYNC" -eq 1 ]] && { say "Обновляю базы"; pacman -Sy; }
  say "Готово: репозиторий $REPO_NAME отключён"
  exit 0
fi

say "Подключаю репозиторий $REPO_NAME"
mkdir -p "$CONF_DIR"
cat > "$REPO_CONF" <<EOF
# Репозиторий vMixRTC (создан vmixrtc-repo.sh)
[$REPO_NAME]
SigLevel = Optional TrustAll
Server = $REPO_URL
EOF
echo "  записан $REPO_CONF"

if grep -qF "$INCLUDE_LINE" "$PACMAN_CONF"; then
  echo "  Include уже есть в $PACMAN_CONF"
else
  backup
  {
    printf '\n# vMixRTC repository (added by vmixrtc-repo.sh)\n'
    printf '%s\n' "$INCLUDE_LINE"
  } >> "$PACMAN_CONF"
  echo "  в $PACMAN_CONF добавлено: $INCLUDE_LINE"
fi

if [[ "$SYNC" -eq 1 ]]; then
  say "Обновляю базы"
  pacman -Sy
fi

say "Готово. Установка: sudo pacman -S vmixrtc-bin"
