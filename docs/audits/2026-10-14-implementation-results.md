# Fusion Launcher: результаты реализации Lifecycle / Ownership и Network / Trust

> Исторический отчёт. Актуальные результаты на зафиксированном SHA, настоящем libtorrent и установленном Windows-sidecar: [2026-10-15-acceptance-results.md](2026-10-15-acceptance-results.md). Указанные ниже ограничения и число тестов описывают состояние до последующей native-проверки.

Дата: 2026-10-14. База: `2ff0ec4505fe463583678609b0fd8efe061ba727`.
Изменения находятся в рабочем дереве; коммиты и публичный релиз не создавались.
Основание: согласованное задание `2026-10-13-implementation-brief.md` и замечания двух внешних рецензентов.

## Реализовано

### Lifecycle / Ownership

- Общий координатор резервирует subject до создания файлов. Отмена наблюдаема во время ожидания HTTP/DNS/headers/body; удаление ждёт последнего владельца, включая `spawn_blocking` и открытые дескрипторы. Таймаут не даёт права удалить файлы или запустить конкурирующего владельца.
- SQLite хранит `attempt_id` и `run_id` (миграция v3). Progress, handle, terminal callbacks проверяют поколение и разрешённый переход. Завершение создаёт первую запись загрузки атомарно и не воскрешает удалённую попытку.
- Выбор queued torrent, условное получение права на запуск и публикация runtime выполняются под одним коротким lock. Сохранённые process-local torrent ID сбрасываются после перезапуска.
- Sidecar имеет подтверждение выхода для конкретного поколения. Observer установлен и для ранних отказов запуска. Закрытие event stream без `Terminated` не считается доказательством выхода. Pause/stop сохраняют payload; forced kill не обещает сохранения fastresume.
- Новые установки получают исключительно созданный каталог и trusted ownership в SQLite. Маркеры из файлов/архива не предоставляют права удаления. Очистка удаляет только точные известные identities и пустые собственные каталоги; рекурсивного удаления пользовательского target нет.
- История trusted manifests сохраняется при смене каталога/попытки и учитывается при удалении. Повторный snapshot отвергает подмену известного файла. Неизвестные добавления после первого snapshot остаются чужими.
- Windows guards удерживают ancestors от подмены. HTTP публикует проверенный файл через тот же открытый handle с запретом замены destination. Повторно открытый staging/archive заново хешируется через неизменяемый дескриптор. Удаление обработанного архива привязано к handle. Распаковка удерживает ancestors конкретного output и использует `create_new`.
- `.part` сохраняется при отмене/ошибках для retry либо trusted cleanup; удаление по повторному поиску имени исключено. HTTP 200 после Range сбрасывает hash, offset и счётчики; 206 проверяет Content-Range.
- Frontend отбрасывает поздние события удалённых/сменившихся attempts и прежних runs. Общий фильтр используется также модальным окном. API удаления возвращает `changed` и `preservedPaths`, остатки сообщаются через существующие панели сообщений.

Основные новые модули: `download_operations.rs`, `download_installation.rs`, `install_ownership.rs`, `download_http.rs`, `storage/operations.rs`, `torrent_runtime.rs`, `commands/game_download_runtime.rs`, `commands/download_removal.rs`.

### Network / Trust

- Внешние GET используют проверяемые ручные redirects: относительные Location, предел переходов, обнаружение циклов, запрет userinfo и HTTPS downgrade. Передаются только явно разрешённые Range/If-Range, без origin credentials.
- DNS-политика встроена в фактический connector прямого запроса. Ошибка/пустой ответ и смешанный public/private ответ отклоняются. Сохраняются hostname/TLS, поддержка системного proxy и Fake-IP.
- JSON ограничен 16 MiB, artwork — 20 MiB, включая chunked ответы без Content-Length. Fetch deadline включает чтение тела; у крупных download stream отдельный idle timeout без общего ограничения длительности.
- Preview и запись репозитория используют одинаковый effective trust: внешний self-declared `official` понижается до community, встроенный источник остаётся official. Enum не расширялся.

## Проверки

| Проверка | Результат |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 263/263 passed |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | passed |
| Cargo fmt и отдельный rustfmt для include-файлов | passed |
| `npm run check` | TypeScript, 98/98 Node tests, static check и production static build passed |
| Python sidecar lifecycle unittest | 4/4 passed |
| `npm run mvp:smoke` | 5/5 passed |
| `npm run mvp:visual` | passed; desktop 1280/1365/1440/1920, library, downloads, setup, settings |

Скриншоты: `.tmp/mvp-visual/`. Центр загрузок и карточка игры просмотрены визуально; новые визуальные роли и screenshot baselines не утверждались.

Регрессии покрывают A→delete→B→late A, старый run после pause/resume, двойной queue claim, отмену во время blocked add, неподтверждённый stop, чужой текущий process ID после restart, sentinel и поддельный marker, junction, замену file/root identity, source/destination race при публикации, pause→sentinel→resume→remove, stalled headers/body, Range 200 и первую атомарную completion-запись.

При полном прогоне обнаружена гонка самого тестового HTTP fixture: accepted socket на Windows наследовал nonblocking listener. Сервер теперь явно переводит accepted socket в blocking mode и читает headers полностью. Производственная сетевая политика для прохождения теста не ослаблялась.

Повторный Rust/Tauri review после исправлений не выявил новых блокирующих дефектов в проверенных изменениях.

## Практические ограничения

- Проверена Windows-реализация. Handle-bound publication на других ОС сейчас отклоняется безопасно; переносимость не подтверждена.
- После закрытия inventory новые пути не присваиваются автоматически. Если resumed torrent создаст ранее неизвестный payload, удаление сохранит его и сообщит остаток. Для полного удаления таких новых результатов нужен подтверждённый inventory производителя.
- Обычный system asset переносится из owned staging в требуемый путь эмулятора. Этот финальный путь пока не входит в ownership staging и сохраняется/сообщается при удалении. Существующие пользовательские файлы не заменяются; redownload также не удаляет их заранее.
- Системный proxy и Fake-IP остаются доверенной сетевой инфраструктурой; проверка локального DNS не доказывает адрес назначения, выбранный удалённым proxy. Fixed-provider GitHub resolver сохраняет отдельную прежнюю политику.
- TorrentManager проверен с управляемым fake engine; stdin shutdown — Python unit tests. Полная packaged Tauri + настоящий libtorrent загрузка/сохранение fastresume и native emulator acceptance не выполнялись. Эти результаты не являются разрешением на публичный релиз.
