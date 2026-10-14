# Fusion Launcher: Windows acceptance и пакет для рецензентов

Дата: 2026-10-15. Проверенный исходный код: `5922db4bb1b0af8371493aa3fe2a5992c10c1d56`, ветка `wip/lifecycle-network`. Публичный релиз не выполнялся.

Этот отчёт заменяет сведения о текущем состоянии из `2026-10-14-implementation-results.md`: тот документ описывал незакоммиченное дерево до настоящих native-проверок. Здесь результаты привязаны к одному снимку кода. Отчёт и его последующий документационный коммит не меняют проверенный runtime.

## Снимки и воспроизводимость

1. `3f59978` — требуемый WIP-снимок Lifecycle/Ownership + Network/Trust. Первичный clean-SHA QA: 263 Rust и 98 Node tests.
2. `162b06d` — сообщения об ошибках DNS/прокси и сохранение цепочки причин сетевого отказа.
3. `663a94d` — исключение старого fallback executable из состава обычного установщика.
4. `5922db4` — Windows process tree, поздний torrent inventory, реальные fastresume-регрессии и проверки освобождения handles.

Сырые логи окончательного прогона находятся в `.tmp/acceptance/5922db4/`. `sha.txt`, `commits.txt`, `status-before-package.txt`, `status-after.txt` и `diff-after-wip-stat.txt` фиксируют снимок, чистоту дерева и изменения после WIP. Пустой status-файл означает чистое дерево. Артефакты сборки и QA исключены из Git.

## Автоматический QA на Windows

| Проверка | Результат | Сырой лог |
| --- | --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 277 passed, 0 failed, 3 ignored | `qa-cargo.txt` |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | PASS | `qa-clippy.txt` |
| Cargo fmt и rustfmt для существующих include-файлов | PASS | `qa-fmt.txt` — пустой вывод при успехе |
| `npm run check` | TypeScript, 98 Node tests, static check, Next static build PASS | `qa-npm.txt` |
| Python sidecar lifecycle | 18/18 PASS | `qa-python.txt` |

Три ignored Rust-теста требуют собранного PyInstaller executable и настоящего libtorrent; они запускаются отдельно. Windows CI уже выполняет Rust tests/clippy/format на `windows-latest`. Сам факт наличия workflow не означает, что удалённый CI был запущен в этой работе.

## Изменения, проверенные реальными процессами и файлами

### Остановка PyInstaller onefile

`scripts/build-sidecar.mjs` действительно собирает `--onefile`. Поэтому sidecar создаётся сразу внутри отдельного не наследуемого Windows Job Object с `KILL_ON_JOB_CLOSE`, без возможности breakaway. Назначение job происходит атомарно при `CreateProcessW` через `PROC_THREAD_ATTRIBUTE_JOB_LIST`, до выполнения кода ребёнка. Force-stop вызывает `TerminateJobObject`, а не завершает один PID.

Подтверждение завершения относится к `engine_id` и `generation` и требует одновременно наблюдаемого выхода корневого процесса и нулевого `ActiveProcesses` в job. Потеря event stream не предоставляет разрешение на cleanup. Observer установлен и для отказов между созданием процесса и обычным event loop.

Один параллельный повтор настоящих onefile-тестов во время release-компиляции дал таймаут ожидания process handle через 10 секунд: `qa-native-onefile-job.txt`. Это реальное неподтверждённое завершение в тот момент. Причина не установлена; нагрузка/отложенное завершение I/O — гипотеза, а не доказанный диагноз. Последовательный повтор обоих тестов прошёл: `qa-native-onefile-job-serial.txt`. Неуспешный лог сохранён, таймауты не увеличивались ради прохождения теста.

Повтор в исходном параллельном режиме также прошёл (`qa-native-onefile-job-parallel-repeat.txt`). На установленном финальном sidecar оба теста прошли ещё раз (`qa-native-installed-job.txt`): каждый retained process handle сигнализировал выход, job опустел; после убийства launcher-fixture bootloader и Python-ребёнок завершились.

Отдельная проверка кода подтвердила безопасный отказ: таймаут в `LibtorrentEngine::stop_locked` возвращает ошибку, `TorrentManager::stop_download` не освобождает guards, а `download_removal` не доходит до cleanup. Такой отказ ухудшает доступность операции, но не разрешает удаление при потенциально живом writer.

### Владение торрент-файлами после metadata

Payload до согласования inventory заблокирован через `upload_mode`, нулевые priorities и `default_dont_download`. После metadata Python передаёт точные непаддинговые пути и путь fastresume. Rust исключительно создаёт новые файлы/каталоги, проверяет identities известных файлов, сохраняет inventory в SQLite и устанавливает guards; только затем отправляет `inventory_approved`.

Inventory может пополняться подтверждёнными путями производителя после первоначального пустого snapshot. Уже существующий неизвестный файл не усыновляется. Для создания Windows-каталогов применяется handle-relative `NtCreateFile(FILE_CREATE)`: успешное создание не отделено от получения identity окном подмены каталога.

Учитываются padding-файлы hybrid torrents: они сохраняют priority 0, обычные файлы получают priority 4. Переход к скачиванию выполняется после pause native connections и применения priorities, затем снимается upload-only режим и выполняется resume. Это исправило реальное зависание повторного подключения, которое не воспроизводил fake engine.

### Fastresume

Сохранение включает metadata (`save_info_dict`); при загрузке восстанавливаются discovery peers/trackers из исходного magnet. Недоверенные mappings путей из fastresume отвергаются до `add_torrent`, хеши сверяются с magnet, `seed_mode` сбрасывается. При непригодном resume используется исходный magnet.

Normal pause/stop пытается сохранить fastresume и подтверждает завершение. Force-kill не гарантирует сохранение. `delete_files=false` означает, что Fusion не удаляет существующие payload/resume-данные; это не обещание успешного обновления resume при любой ошибке диска или аварии процесса.

Native fixture использует libtorrent 2.0.11, PyInstaller 6.11.1 и локальную раздачу уникальных случайных данных, созданных тестом. Коммерческие игровые данные не используются. Проверяется hybrid/v2 torrent с padding и пустым файлом. Перед pause скачивается минимум 100 MiB; baseline берётся из окончательных `engine.stats(1)` после подтверждённого stop; в SQLite отдельно проверяется статус `paused`. Resume обязан получить минимум 8 MiB сверх окончательного baseline, поэтому уже принятые перед pause данные не могут дать ложный PASS.

Два предыдущих сильных повтора того же runtime: `108267940 → 120719780` байт за 4.043 с и `108169636 → 120654244` байт за 3.515 с после resume. Оба подтвердили удаление installation/library без остатков и отсутствие writer после stop. Они сохранены как дополнительные логи, но окончательная приёмка выполняется отдельно на установленном sidecar проверенного SHA.

Окончательный native-прогон на установленном sidecar: `108822528 → 117686272` байт за **3.560 с** после resume, статус SQLite `paused` перед возобновлением, успешный rename библиотеки на паузе, подтверждённая остановка и удаление installation/library без остатков. Результат: **1/1 PASS**, `qa-native-installed-manager.txt`. Вместе с двумя installed-onefile тестами отдельно выполнены все три ignored native-теста из Cargo suite. После их завершения matching sidecar-процессов не осталось (`sidecar-processes-after.txt`).

### Handles и удаление

HTTP tests проверяют success, cancellation, size error и panic: lease живёт до завершения writer, затем rename/remove родителя проходят. Torrent tests проверяют completion/error/cancel/panic join; native pause дополнительно переименовывает библиотеку, затем resume продолжает работу. При неизвестном исходе stop guards намеренно сохраняются.

Для legacy/imported installation автоматическое усыновление по форме пути не вводилось. Без trusted inventory приложение сохраняет файлы и возвращает понятную причину: имя каталога не доказывает владение; распознанные файлы нужно удалить вручную. Это выбранная ветвь критерия «удаляется либо сообщает причину отказа». System assets за пределами trusted staging также сохраняются с объяснением.

## Сеть

Внешние GET используют fail-closed connector: ошибка/пустой DNS, приватный адрес и смешанный public/private ответ отвергаются. Сообщения теперь называют DNS и системный proxy, сохраняют цепочку причин. Проверки Fake-IP и proxy fixtures покрыты автоматическими тестами.

Fixed-provider GitHub resolver сохраняет отдельную прежнюю политику: allowlisted HTTPS GitHub/GitHubusercontent и ограниченные redirects. Pinning применяется к первоначальному `api.github.com` при успешном допустимом DNS; redirect policy проверяет HTTPS, host allowlist и число переходов, но не выполняет независимый DNS pinning каждого redirect. При ошибке/пустом DNS первоначальный resolver допускает дальнейшее разрешение reqwest/proxy. Эта совместимость не распространяется на произвольные внешние GET. Системный proxy/TUN остаётся доверенной инфраструктурой, а локальный DNS не доказывает адрес назначения, выбранный удалённым proxy.

Ручная матрица провайдерского DNS, системного proxy без TUN и Clash/mihomo TUN с fake-ip на настоящих пользовательских конфигурациях не выполнена.

## Собранный и установленный пакет

Production onefile пересобран из проверенного дерева: `qa-sidecar-build.txt`. Затем Tauri/NSIS собран с отдельным acceptance overlay: `qa-package-build.txt`. Installer **Fusion Launcher Acceptance_0.4.0_x64-setup.exe** имеет отдельные productName и identifier (`app.fusion.launcher.acceptance`), updater artifacts выключены. Это локальный кандидат для проверки, не опубликованный релиз.

Он установлен/обновлён в `F:\MVP\Fusion Launcher\.tmp\acceptance\installed`. Обычная пользовательская установка `C:\Users\admin\Downloads\Fusion Launcher` не обновлялась; хеш её EXE до/после совпадает. Registry snapshots и контрольные хеши включены в evidence.

| Установленная проверка | Результат |
| --- | --- |
| Silent NSIS install | exit 0 |
| Backend package smoke | exit 0 |
| Installed sidecar `--help` | exit 0 |
| Настоящий TorrentManager → установленный libtorrent sidecar | PASS, новые данные после fastresume, удаление без остатков |
| Onefile force-stop / launcher crash | 2/2 PASS на установленном sidecar |

Текущее содержимое: `fusion-launcher.exe`, `fusion-torrent.exe`, `uninstall.exe`, `profiles/`. Генерируемый `installer.nsi` включает только production sidecar. После переноса fallback из `src/bin` Tauri больше не добавляет старый `fusion_torrent.exe` в новый пакет.

Однако NSIS-обновление предыдущей тестовой установки оставило этот старый EXE на диске. Он не входит в новый installer и был явно удалён только из нашей изолированной установки после проверки абсолютного пути и совпадения известного размера. Это отражено в `qa-installed.txt`; автоматическая очистка исторических лишних binaries при обновлении этим не доказана. Чистая установка на чистой Windows не выполнялась.

Артефакт installer сохранён в `.tmp/acceptance/5922db4/`, рядом с сырыми логами. SHA256 source, built/installed sidecar, установленного launcher, installer, test executable и acceptance config записаны в `sha256-artifacts.txt`; built и installed sidecar имеют одинаковый SHA256 `2FBB75D077C19A1D98C2F1B74A74E4B39F9E5998A28642BEAFC927E5FC3A22D1`. Версии инструментов находятся в `versions.txt`.

Review ZIP содержит запрошенные `install_ownership.rs`, `commands/download_removal.rs`, `torrent_runtime.rs`, `scripts/build-sidecar.mjs`, дополнительные runtime/test файлы, workflow, сырые логи, diff stat и сведения о SHA. Installer предоставляется отдельно, чтобы пакет исходников и результатов оставался небольшим.

## Границы приёмки

- Проверка проводится на текущей Windows-машине, не в чистой VM. Установка VC runtime на чистом компьютере, обновление пользовательской установки и GUI acceptance этим не доказаны.
- Реальная раздача запускает production `TorrentManager` и production Windows spawn/event loop из Rust test executable, используя установленный onefile sidecar. Это не полный GUI Tauri сценарий.
- Backend package smoke использует собственные fixtures и тестовый emulator executable. Полная цепочка с поддерживаемым настоящим эмулятором и легальной игрой остаётся отдельной приёмкой.
- Archive ownership/cleanup покрыт автоматическими проверками, но установленный GUI сценарий «архивная игра → удалить → каталога нет» отдельно не выполнен.
- Non-Windows handle-bound publication, ownership конечных system assets и ручная сетевая матрица остаются ограничениями.
- Во время активной записи библиотека защищена от rename/delete. После подтверждённого pause и терминальных состояний guards освобождаются; неподтверждённый stop оставляет блокировку намеренно.
- Перевод существующих `include!` в `mod` не включён в эту приёмку. Дополнительно выявлен прежний dev-only дефект: fallback-вызов cargo в `build-sidecar.mjs` не передаёт обязательный `rqbit-fallback` feature; production PyInstaller build его не использует.

Публичная готовность не объявляется. Следующие критерии закрытого MVP: чистая Windows/VM, GUI цепочка с настоящим эмулятором и легальной игрой, пользовательская сетевая матрица и повторная проверка остановки под нагрузкой.
