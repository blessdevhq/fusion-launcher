# Как собрать свой источник игр (source library)

Лаунчер не хранит игры — он подключает **источники**: JSON-файлы, описывающие каталог.
Если в записи указана прямая ссылка (`http`/`magnet`), в каталоге появляется кнопка
**Download** → **Play**. Если ссылка `user_provided` — кнопка **Import** (выбрать свой файл).

Готовые источники:

- [`public/source-libraries/official-starter.json`](../public/source-libraries/official-starter.json) — официальный starter: first-party demo + openly licensed homebrew, можно сразу вставлять в лаунчер:
  `https://blessdevhq.github.io/fusion-launcher/source-libraries/official-starter.json`
- [`public/source-libraries/unofficial-player-wishlist.json`](../public/source-libraries/unofficial-player-wishlist.json) — неофициальный wishlist для promo/showcase: известные коммерческие игры только как `user_provided`, без ссылок на ROM/BIOS/firmware/keys/artwork.
  URL для проверки в лаунчере:
  `https://blessdevhq.github.io/fusion-launcher/source-libraries/unofficial-player-wishlist.json`
- [`examples/repositories/homebrew-library.json`](../examples/repositories/homebrew-library.json) —
  каталог бесплатного homebrew с прямыми ссылками на релизы авторов.

## Самый быстрый путь: скрипт

Скрипт сам скачивает файл, считает `sha256` и размер (они обязательны для `http`),
и выдаёт готовую запись. Без `--into` — печатает запись в консоль; с `--into` —
вставляет её в существующий источник и проверяет валидатором.

```bash
# Напечатать запись (скопировать вручную):
npm run source:add -- \
  --url "https://example.org/releases/cool-game.nes" \
  --platform nes \
  --title "Cool Game" \
  --developer "Author" \
  --genres "Action,Homebrew"

# Сразу вставить в источник и проверить:
npm run source:add -- \
  --url "https://example.org/releases/cool-game.nes" \
  --platform nes --title "Cool Game" \
  --into examples/repositories/homebrew-library.json
```

Флаги: `--url` `--platform` `--title` (обязательные), `--id` `--profile`
`--developer` `--genres` `--year` `--magnet` `--into` (опциональные).
`--profile` и расширения подставляются по платформе автоматически.

## Поддерживаемые платформы (с готовым профилем эмулятора)

| `--platform` | Профиль | Расширения |
|---|---|---|
| `nes` | `nes-mesen` | `.nes` |
| `snes` | `snes-mesen` | `.sfc`, `.smc` |
| `n64` | `n64-rmg` | `.z64`, `.n64`, `.v64` |
| `gba` | `gba-mgba` | `.gba` |
| `ps2` | `ps2-pcsx2` | `.iso`, `.chd` |
| `psp` | `psp-ppsspp` | `.iso`, `.cso` |

## Структура файла вручную

```jsonc
{
  "metadata": {
    "id": "my-library",          // уникальный, НЕ "fusion-launcher-demo"
    "name": "My Library",
    "version": "1.0.0",
    "schemaVersion": 3,
    "trustLevel": "community"     // official | community | unknown
  },
  "system_files": [],            // BIOS/ключи — обычно user_provided
  "catalog": [
    {
      "id": "cool-game",
      "platform": "nes",
      "title": "Cool Game",
      "contentMode": "downloadable",
      "setupProfileId": "nes-mesen",
      "downloads": [
        {
          "kind": "http",
          "url": "https://example.org/cool-game.nes",
          "sha256": "<64 hex символа>",   // ОБЯЗАТЕЛЕН для http
          "sizeBytes": 40976              // опционально, проверяется
        }
      ],
      "expectedExtensions": [".nes"]
    }
  ]
}
```

Вариант с torrent вместо `http`:
```jsonc
"downloads": [ { "kind": "magnet", "uri": "magnet:?xt=urn:btih:..." } ]
```
У `magnet` хеш не нужен — целостность гарантирует BitTorrent.

### Важные правила

- **Ссылка должна вести на сам файл игры** (`.nes`/`.gba`/…), не на `.zip`.
  Загруженный файл игры не распаковывается (распаковка есть только для эмуляторов).
- Для `http` **`sha256` обязателен** — иначе валидатор и загрузчик отклонят источник.
- `id` уникальны в пределах каталога; пустые строки запрещены (опускайте поля).
- `bundled`-источники разрешены только встроенному демо — для своих используйте `http`/`magnet`.

## Проверка и подключение

```bash
npm run source:validate -- examples/repositories/my-library.json
```
`[ok]` — источник готов. Подключение в приложении: **Settings → Sources** →
выбрать локальный `.json` либо вставить URL (источник можно опубликовать,
например, на GitHub Pages, и раздавать ссылкой).

## Где брать легальный контент

Используйте только то, что автор разрешил распространять: homebrew и public-domain.
Надёжный источник прямых ссылок — **GitHub Releases** авторов (см. пример-каталог:
RoboRun, µCity Advance, Skyland, Blind Jump, Sym Merged — все с открытыми лицензиями).
Не наполняйте каталог пиратскими копиями коммерческих игр.
