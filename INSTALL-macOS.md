# Установка vMixRTC на macOS

Сборки для macOS раздельные, выбирайте по своему процессору:

| процессор | файл |
|---|---|
| Apple Silicon (M1–M4) | `vMixRTC-macos-arm64.zip` |
| Intel | `vMixRTC-macos-x86_64.zip` |

Проверить, что у вас, можно командой `uname -m`: `arm64` — Apple Silicon, `x86_64` — Intel.

## Если сборка подписана Developer ID

Скачайте `.dmg`, откройте, перетащите **vMixRTC** в «Программы» — и запускайте. Никаких
предупреждений не будет.

## Если сборка подписана ad-hoc (без Apple Developer ID)

Приложение работает, но Gatekeeper его не знает: macOS скажет, что разработчик не проверен
(«…нельзя открыть, так как Apple не может проверить его на наличие вредоносного ПО»).

**Способ 1 — без терминала.** Перетащите `vMixRTC.app` в «Программы», затем:

1. Кликните по приложению **правой кнопкой** (или Control+клик) → **Открыть**;
2. в диалоге ещё раз **Открыть**.

После этого macOS запоминает выбор, и дальше приложение запускается обычным двойным щелчком.
На новых версиях macOS тот же диалог доступен через **Системные настройки → Конфиденциальность
и безопасность → «Всё равно открыть»**.

**Способ 2 — одной командой в Терминале.** Снимает метку карантина со скачанного приложения:

```bash
xattr -dr com.apple.quarantine /Applications/vMixRTC.app
```

**Способ 3 — Homebrew** (см. раздел ниже): cask сам снимает карантин при установке.

## Проверить, что скачалось целиком

```bash
shasum -a 256 ~/Downloads/vMixRTC_*.dmg                   # сверьте с суммой из релиза
spctl -a -vv /Applications/vMixRTC.app                    # «accepted, adhoc» или «source=Developer ID»
file /Applications/vMixRTC.app/Contents/MacOS/vmixrtc          # ожидаем arm64 или x86_64 — по вашей сборке
```

## Homebrew

```bash
brew tap fursyt12/vmixrtc https://github.com/fursyt12/homebrew-vmixrtc
brew install --cask vmixrtc
```

Cask лежит в `packaging/homebrew/vmixrtc.rb` — скопируйте его в свой tap-репозиторий
`homebrew-vmixrtc` (файл `Casks/vmixrtc.rb`) и обновляйте `version` при релизах.
В cask добавлен `postflight`, который снимает карантин: иначе Gatekeeper не даст запустить
неподписанное приложение.

## Про Apple Developer ID: платно ли и долго ли

* **Членство Apple Developer Program — $99 в год** (индивидуальное или компания). Именно оно даёт
  сертификат **Developer ID Application** и доступ к нотаризации (`notarytool`). Без членства
  подписать сборку так, чтобы Gatekeeper доверял ей на чужих Mac, невозможно: самодельный
  сертификат действует только на вашей машине.
* **Сроки:** регистрация физлица обычно подтверждается за 1–2 дня (иногда быстрее); для компании
  нужен D‑UNS и это может занять от нескольких дней до пары недель. Нотаризация каждой сборки —
  минуты (обычно 1–10), настраивается один раз.
* **Обходной путь без оплаты:** ad-hoc подпись + инструкция выше (или Homebrew cask). Это рабочий
  вариант для сообщества; минус — предупреждение Gatekeeper при первом запуске.
* **Для некоммерческих/образовательных организаций** у Apple есть программа освобождения от
  взноса (fee waiver) — если это ваш случай, стоит подать заявку.

## Как включить подпись в CI, когда членство появится

Сборка уже готова к этому: workflow `.github/workflows/rust.yml` сам подписывает и нотаризует
`.dmg`, если в репозитории появились секреты:

| секрет | что это |
|---|---|
| `APPLE_CERTIFICATE` | сертификат Developer ID Application в base64 (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | пароль от `.p12` |
| `APPLE_SIGNING_IDENTITY` | имя подписи, например `Developer ID Application: Иван Иванов (TEAMID)` |
| `APPLE_ID` | Apple ID разработчика |
| `APPLE_PASSWORD` | app-specific password из appleid.apple.com |
| `APPLE_TEAM_ID` | Team ID из developer.apple.com |

Пока секретов нет — используется ad-hoc (`APPLE_SIGNING_IDENTITY: "-"`), и в логе сборки
печатается, какой режим выбран.
