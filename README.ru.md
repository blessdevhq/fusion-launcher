# Fusion Launcher

[English version](README.md)

Fusion Launcher - Windows-first лаунчер для библиотек игр, готовых к запуску
через эмуляторы. Приложение подключает bring-your-own репозитории, валидирует
каталог, отслеживает установку файлов и запускает игры через настроенные или
автоматически подготовленные эмуляторы.

Проект построен на Next.js, Tauri 2, Rust и локальном хранилище.

## Возможности

- Подключение source-library JSON каталогов из GitHub Pages, HTTPS URL или
  локальных файлов.
- Валидация schema v3 каталогов с metadata, artwork, тегами, жанрами, setup
  profiles и требованиями к установке.
- Встроенный first-party NES smoke-test репозиторий для безопасной проверки
  первого запуска и релизов.
- Отслеживание direct, bundled и torrent-aware загрузок.
- Импорт игр, BIOS, firmware и keys, которые пользователь предоставляет сам.
- Metadata и artwork из source libraries, ScreenScraper и SteamGridDB, если они
  настроены.
- Preflight-проверки перед запуском эмулятора.
- Diagnostics, health checks, GitHub Releases update checks и Windows package
  smoke tests.

## Первый запуск

Самый быстрый путь на Windows - встроенная demo-настройка:

1. Установи Fusion Launcher.
2. Открой приложение и выбери **Set up demo**.
3. Fusion Launcher подключит встроенный demo-источник, подготовит поддерживаемый
   NES emulator, установит first-party smoke ROM и включит **Play Demo**.

Автоматическая portable-настройка эмуляторов доступна для NES, SNES, Nintendo
64, Game Boy Advance, PlayStation 2 и PSP. PlayStation 1 и Nintendo Switch пока
используют ручной выбор executable. BIOS, firmware и keys всегда остаются
пользовательскими файлами.

## Модель контента

Fusion Launcher не поставляет коммерческие ROM, BIOS, firmware, keys или
third-party игровые payloads. Пользователи и авторы source libraries отвечают за
то, что используют только контент, на который у них есть права.

Файл `public/demo-content/fusion-launcher-smoke.nes` - first-party smoke-test
контент для проверки лаунчера. Условия описаны в
`public/demo-content/LICENSE.txt`.

## Source Libraries

Source libraries - это JSON каталоги, описывающие игры, платформы, metadata,
artwork, setup profiles и требования к установке.

Полезные ссылки:

- [Source library template](docs/source-library-template.md)
- [Repository authoring guide](docs/repository-authoring.md)
- [Rich metadata example](examples/repositories/showcase.metadata.json)

Starter template опубликован по адресу:

```text
https://mrbeastie.github.io/fusion-launcher/source-library-template/repository.json
```

## Разработка

Требования:

- Node.js 22
- Rust stable
- Windows build tools для Tauri desktop builds

Установка зависимостей:

```powershell
npm ci
```

Стандартные проверки:

```powershell
npm test
npm run typecheck
npm run static-check
npm run source:template:check
npm run rust:test
```

Полный local QA gate:

```powershell
npm run qa
```

Запуск web shell:

```powershell
npm run dev
```

Сборка Windows desktop app:

```powershell
npm run tauri:build
```

Валидация source library:

```powershell
npm run source:validate -- templates/source-library/repository.json
```

## Release

Windows release workflow запускается на тегах `vX.Y.Z` и загружает:

- NSIS installer
- updater zip
- updater signature
- `latest.json` для Tauri updater

Для релизной сборки нужен `TAURI_SIGNING_PRIVATE_KEY`; если ключ зашифрован,
также нужен `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Локальный Windows release smoke gate:

```powershell
npm run mvp:release:windows
```

Документация:

- [MVP Windows install checklist](docs/mvp-windows-install.md)
- [Release checklist](docs/release-checklist.md)
- [Metadata and artwork notes](docs/metadata-artwork.md)

## Совместимость

Раньше проект назывался RetroHydra. Новые установки используют названия,
идентификаторы и файлы Fusion Launcher. Legacy `RETROHYDRA_*` env vars, старые
имена баз данных и встроенные demo identifiers всё ещё поддерживаются как
fallback для существующих установок и CI.

## License

Пока исходный код не распространяется по отдельной open-source лицензии.
Репозиторий публичен для MVP review и validation, если отдельный license file не
будет добавлен. Demo smoke-test content покрыт отдельно в
`public/demo-content/LICENSE.txt`.
