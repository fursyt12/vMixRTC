# Arch Linux: пакеты vMixRTC

Здесь два PKGBUILD для AUR:

| каталог | пакет | что делает |
|---|---|---|
| `vmixrtc-bin/` | **vmixrtc-bin** | готовая сборка: скачивает компактный артефакт релиза (`vMixRTC-linux-x86_64-pkg.tar.zst`, ~5 МБ) и распаковывает установочное дерево |
| `vmixrtc/` | **vmixrtc** | сборка из исходников тега `rust-v$pkgver` |

Пакеты не конфликтуют по содержимому: `vmixrtc-bin` объявляет `provides=('vmixrtc')` и
`conflicts=('vmixrtc')`, у `vmixrtc` обратный `conflicts=('vmixrtc-bin')` — одновременно
установить можно только один.

Зависимости (проверены на Arch):

```
depends=('webkit2gtk-4.1' 'gtk3' 'alsa-lib' 'systemd-libs')
optdepends=('ndi-sdk: приём NDI и список источников (проприетарный runtime)')
```

Файлы, которые ставит пакет: `usr/bin/vmixrtc`, `usr/bin/vmixrtc-cli`,
`usr/share/applications/vmixrtc.desktop`, иконки в `usr/share/icons/hicolor/*`,
`usr/share/licenses/vmixrtc/LICENSE`.

## Две вещи, на которых легко споткнуться

1. **`-flto` в CFLAGS ломает сборку.** `makepkg` добавляет `-flto=auto` в `CFLAGS`, а C-часть
   crate `ring` собирается через `cc`; LTO-объекты потом не линкуются с Rust-кодом и линковка
   падает с `undefined symbol: ring_core_0_17_14__*`. Поэтому в PKGBUILD стоит
   `options=('!debug' '!lto')`.
2. **Каталоги функций должны попасть в систему.** Приложению нужны `Functions.xml` и
   `NewFunctions.xml` (459 + 811 функций vMix). Пакеты кладут их в `/usr/share/vmixrtc`, и порт
   ищет их там (плюс рядом с бинарём, в `data/`, в ресурсах macOS-бандла `Contents/Resources/data`
   и в `/usr/lib/vMixRTC/data`, куда их кладут `.deb`/AppImage). Без этих файлов кнопки и скрипты
   не соберут запросы к vMix.

## Сборка из исходников: почему без tauri-cli и node

Фронтенд статический и лежит в `crates/vmixrtc/ui`; `tauri::generate_context!` вшивает его
при обычной сборке, поэтому нужен только `cargo`. Схема PKGBUILD стандартная для Rust:

* `prepare()` — `cargo fetch --locked` с `CARGO_HOME="$srcdir/cargo-home"`;
* `build()` — `cargo build --release --frozen --locked -p vmixrtc -p vmixrtc-cli`
  (thin-LTO + `codegen-units = 1`, поэтому сборка занимает несколько минут);
* `package()` — установка бинарей, ярлыка, иконок и лицензии.

## Свой pacman-репозиторий (AUR не нужен)

Регистрация в AUR временно закрыта, поэтому основной способ установки — собственный
репозиторий, который публикует CI (`.github/workflows/arch-repo.yml`): пакет собирается в
Arch-контейнере, база создаётся `repo-add` и выкладывается в роллинг-релиз `repo-x86_64`.

### Подключение одной командой

В Arch нет аналога `add-apt-repository`: сторонние репозитории подключают правкой
`/etc/pacman.conf`. Чтобы это была одна команда, в релиз кладётся скрипт
`vmixrtc-repo.sh`:

```bash
curl -fsSLO https://github.com/fursyt12/vMixRTC/releases/download/repo-x86_64/vmixrtc-repo.sh
less vmixrtc-repo.sh          # необязательно: посмотреть, что делает
sudo bash vmixrtc-repo.sh     # подключить репозиторий и обновить базы
sudo pacman -S vmixrtc-bin
```

Что делает скрипт (идемпотентно, с резервными копиями):

1. пишет `/etc/pacman.d/vmixrtc.conf` с секцией репозитория;
2. добавляет в `/etc/pacman.conf` строку `Include = /etc/pacman.d/vmixrtc.conf`,
   сохраняя копию файла `pacman.conf.vmixrtc-<дата>.bak`;
3. выполняет `pacman -Sy`.

Повторный запуск ничего не дублирует, `--no-sync` пропускает синхронизацию,
`--remove` отключает репозиторий и убирает свои файлы.

### Вручную

```ini
# /etc/pacman.conf
[vmixrtc]
SigLevel = Optional TrustAll
Server = https://github.com/fursyt12/vMixRTC/releases/download/repo-x86_64
```

Затем `sudo pacman -Sy && sudo pacman -S vmixrtc-bin` (обновление — обычным `pacman -Syu`).

Проверено в контейнере `archlinux:latest` настоящим pacman: после работы скрипта
`pacman -Si vmixrtc-bin` показывает `Repository: vmixrtc`, `Version: 0.1.4-1`,
`Licenses: MIT`, а `pacman -Sw vmixrtc-bin` скачивает пакет из нашего репозитория.

Почему `TrustAll`: пакеты не подписаны GPG-ключом (подпись — возможное улучшение: `repo-add -s`
плюс `SigLevel = Required` и импорт ключа у пользователя). URL роллинг-релиза стабильный,
поэтому при выходе новых версий менять настройку не нужно.

Собрать такой же репозиторий локально:

```bash
cd packaging/arch/vmixrtc-bin
makepkg -f
repo-add vmixrtc.db.tar.zst vmixrtc-bin-*.pkg.tar.zst
# repo-add создаёт vmixrtc.db симлинком; для раздачи файлом:
cp -f --remove-destination vmixrtc.db.tar.zst vmixrtc.db
cp -f --remove-destination vmixrtc.files.tar.zst vmixrtc.files
```

Проверено локально: база содержит запись `vmixrtc-bin 0.1.4-1`, `%SHA256SUM%` в базе совпадает
с самим пакетом, `%FILENAME%` разрешается в существующий файл.

## Публикация в AUR (когда откроют регистрацию)

```bash
# 1. одноразово: создать пакеты в AUR и склонировать их
git clone ssh://aur@aur.archlinux.org/vmixrtc-bin.git
git clone ssh://aur@aur.archlinux.org/vmixrtc.git

# 2. скопировать PKGBUILD и .SRCINFO (он лежит рядом и уже сгенерирован)
cp packaging/arch/vmixrtc-bin/{PKGBUILD,.SRCINFO} vmixrtc-bin/
cp packaging/arch/vmixrtc/{PKGBUILD,.SRCINFO}     vmixrtc/
cd vmixrtc-bin && git add PKGBUILD .SRCINFO
git commit -m "upgpkg: vmixrtc-bin $(grep -oP '^pkgver=\K.*' PKGBUILD)-1" && git push
# то же для vmixrtc
```

Перед публикацией полезно прогнать `makepkg -f` (сборка) и `makepkg -si` (установка) — оба
пакета уже проверены на релизе 0.1.4.

## Обновление версии после релиза (AUR)

```bash
# в обоих каталогах:
sed -i 's/^pkgver=.*/pkgver=<новая версия>/; s/^pkgrel=.*/pkgrel=1/' PKGBUILD
updpkgsums                        # подставит реальные sha256 (сейчас в файлах SKIP)
makepkg --printsrcinfo > .SRCINFO
makepkg -f                        # локальная проверка сборки пакета
git commit -am "upgpkg: ..." && git push
```

`updpkgsums` входит в `pacman-contrib`; `namcap` (проверка раскладки) — отдельный пакет.

## Локальная проверка

```bash
makepkg -f                 # собрать пакет
bsdtar -tf *.pkg.tar.zst   # посмотреть содержимое
makepkg -si                # установить вместе с зависимостями
namcap *.pkg.tar.zst       # придирчивая проверка (по желанию)
```

## Контрольные суммы

В PKGBUILD подставлены реальные `sha256` артефактов релиза `rust-v0.1.4`:

| файл | sha256 (начало) |
|---|---|
| `vMixRTC-linux-x86_64-pkg.tar.zst` | `7ee330ae…` |
| исходники тега `rust-v0.1.4.tar.gz` | `03d9b64f…` |

После нового релиза: поднять `pkgver`, выполнить `updpkgsums`, обновить `.SRCINFO`.

## Что уже проверено (Arch Linux, x86_64)

* `vmixrtc-bin` собирается `makepkg` в пакет **5,2 МБ** (вместо 187 МБ релизного zip);
* `vmixrtc` собирается из исходников тем же `makepkg` (после `options=('!lto')`) и даёт такой же
  набор файлов;
* оба пакета кладут `usr/bin/vmixrtc`, `usr/bin/vmixrtc-cli`, ярлык, четыре иконки hicolor,
  лицензию и **каталоги функций** `/usr/share/vmixrtc/*.xml`;
* `.PKGINFO`: `license = MIT`, `provides = vmixrtc`, зависимости `webkit2gtk-4.1`, `gtk3`,
  `alsa-lib`, `systemd-libs`, `optdepend` на `ndi-sdk`;
* `ldd` на бинаре из пакета — **ноль отсутствующих библиотек**;
* `vmixrtc-cli verify examples` из пакета: **6 файлов, 57 виджетов, 45 команд, 0 проблем**;
* проверен и поиск каталогов: с раскладкой `usr/lib/vMixRTC/data` (как в `.deb`/AppImage) CLI
  находит функции, без них — сообщает понятную ошибку;
* GUI, запущенный прямо из распакованного пакета, открывает окно без паник.

## Идея автоматизации

В CI можно добавить джоб, который после релиза подставляет `pkgver`/`sha256`, генерирует
`.SRCINFO` и пушит в оба AUR-репозитория по SSH-ключу из секрета (`AUR_SSH_KEY`). Тогда
обновление пакетов будет полностью автоматическим.
