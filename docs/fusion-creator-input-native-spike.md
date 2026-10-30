# Поток E: SDL внутри приложения — отчёт интегратору

Статус: **повторный CI с закреплённым Rust 1.95.0 выполняется; готовность спайка пока не объявлена**. Локальный модуль/fixtures/release probe и полный QA Rust 1.95.0 с четырьмя test threads проходят. Предыдущий Windows CI прошёл все тесты и остановился на clippy Rust 1.99.1 вне E; история ниже сохранена. В новом E workflow toolchain совпадает с локальным; lint не отключён.

## Область изменений

- Worktree `F:/MVP/Fusion-Creator-Input-Native`, ветка `creator/input-native`.
- База: `b08bc35`, фиксирующая уже принятые wave-zero контракты поверх E/H foundation. Изменения предыдущего интегратора сохранены отдельным prerequisite commit, а не переписаны в ходе E.
- Новый модуль `src-tauri/src/input/`, **не отдельный crate**. Предыдущий `tools/creator-input-spike` — исторический эксперимент; доказательством выполнения этого задания служат только новые тесты приложения и native app probe.
- Единственное добавление Cargo.toml: Windows-зависимость `sdl2 = { version = "=0.38.0", features = ["bundled", "static-link"] }`. Cargo.lock получает только её транзитивное замыкание и необходимые уточнения существующих ссылок version-compare.
- Общий `lib.rs` изменён только добавлением Windows-модуля и явно запрашиваемого headless probe. Обычный запуск не стартует SDL и не открывает контроллер.
- Новый `.github/workflows/input-spike-windows.yml`. Существующие CI/release workflows не изменены.
- Нет UI, калибровки, Eden ParamPackage, новых production IPC или замены DTO замороженного Creator/input контракта.

## Контракты и интеграция

- Добавлен экспериментальный Rust API `input::InputWorker`, `DeviceInfo`, `ControllerState`, `RawState`, `ReleaseProof`, `ProbeReport` и `probe()`. Это локальная граница feasibility spike, не новые wire/portable DTO.
- Добавлен явный диагностический CLI-флаг Windows-приложения `--input-spike-probe`. Успех означает только перечисление/teardown в этом процессе, не physical input, Eden launch или Creator verification report.
- Замороженные `fusion_input_*` IPC, `fusion-input:state`, sessionId/sequence, Creator DTO, GameManifest, SQLite и i18n **не изменены**. Production IPC остаётся `action_unavailable` до полной реализации E.
- Общие файлы для интегратора: одна добавленная Windows-зависимость Cargo.toml, её additive lock closure, регистрация модуля и ранняя diagnostic ветка в lib.rs. Existing CI/release workflows не перезаписаны; новый workflow принадлежит этому спайку.
- Историческая база `b08bc35` нужна только если принимающая ветка ещё не содержит frozen wave-zero patch. E-code commit — `79d2714`; CI environment fixes — `2719cef`, `d173e7b`. Не cherry-pick-ать повторно уже принятые prerequisite изменения. В main ничего не слито.

## Реальный worker и API спайка

`InputWorker::start → devices → open(instance_id) → read → stop`.

Один именованный worker владеет context, subsystem, event pump и максимум одним GameController. Повторный одновременный start отклоняется. Объекты SDL и указатели никогда не выходят из worker; через канал идут только owned значения. Перечисление не вызывает GameController::open и не выбирает устройство автоматически; это не утверждение о внутренних OS handles драйверов SDL. Open заново разрешает ephemeral instance ID в актуальный device index и проверяет instance после открытия; неизвестная SDL mapping — ошибка, не угадывание раскладки.

Read возвращает SDL logical buttons, четыре signed stick axes, два logical triggers 0..32767, raw buttons/axes/hats. Значения не калибруются. Disconnect — явная ошибка, не фиктивный нейтральный state. Pump дренируется; shell key events не синтезируются.

Stop явно закрывает выбранный GameController и проверяет null native lookup до teardown; после полного уничтожения SDL-стека worker проверяет `SDL_WasInit(0) == 0`, только затем отправляет acknowledgement. Клиент дополнительно выполняет join. Timeout, panic, разрыв канала или неподтверждённое закрытие не возвращают успех. Drop отключает command channel и join-ит worker; drop не выдаёт release receipt. Join не является механизмом принудительного прерывания зависшего native driver.

`sdl2 0.38.0` имеет важную особенность: `Joystick::drop` пропускает SDL_JoystickClose при detached. Поэтому raw-данные читаются через **заимствованный GameController-owned joystick**, без второго raw open/ref. Тест disconnect проверяет закрытие и после detach.

## Fixtures без физического контроллера

Виртуальное устройство создаётся через настоящий `SDL_JoystickAttachVirtualEx`; setters используют `SDL_JoystickSetVirtual*`, чтение — обычный API worker. Нет mock provider и SDL_PushEvent. Synthetic mapping/VID/PID существуют исключительно под `cfg(test)` и не компилируются в application probe.

- Кнопка A/raw 0: press/release.
- Hat: neutral, все четыре стороны и четыре диагонали; проверяется raw mask и logical D-pad.
- Все четыре stick axes: MIN, -1, 0, 1, MAX.
- Оба trigger: raw MIN/0/MAX → logical 0/16383/32767.
- Неизвестный instance, read без selection, второй open/worker — ошибки.
- Stop при живом handle, stop после disconnect, drop клиента и повторный start.

RED до реализации: `rustc --test --edition 2021 src-tauri/src/input/mod.rs` отклонил fixtures из-за отсутствующих InputWorker/probe. GREEN: `cargo test --manifest-path src-tauri/Cargo.toml --offline --lib input::tests -- --nocapture` — **3 passed, 0 failed, 0 ignored**, fixtures выполнились за **0,41 с**. Полный первый test build — **268,196 с**, с пересборкой SDL и части существующих Tauri-зависимостей. Native version **2.26.4**, после teardown flags **0**. Лог `.tmp/input-fixtures-first.log`.

## Windows-сборка и воспроизведение

Локально: Rust/Cargo 1.95.0, `x86_64-pc-windows-msvc`; Visual Studio Build Tools 2022 17.14.33 с VC x86/x64 и Windows SDK; CMake 3.31.6-msvc6. CMake compiler metadata подтверждает **MSVC 19.44.35227.0**, compiler tools path **14.44.35207** и generator **Visual Studio 17 2022**. Установленный CMake отсутствовал в обычном PATH, использован executable Visual Studio BuildTools. Rust wrapper 0.38.0 и native SDL — разные версии; версия SDL фиксируется probe, не угадывается по версии Rust crate.

```bash
export PATH='/c/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin':"$PATH"
# CARGO_TARGET_DIR необязателен: локально переиспользован общий build cache.
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib input::tests -- --nocapture
npm run qa
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin fusion-launcher --timings
```

Headless diagnostic в **настоящем** `.exe`: `fusion-launcher.exe --input-spike-probe`. Он перечисляет устройства без auto-open и возвращает JSON с native SDL version и release proof, либо exit 1. Это намеренно достижимый application path, чтобы линкер не отбросил весь SDL как неиспользуемый модуль. Он не запускает GUI, не сохраняет файлы/привязки и не сообщает об успешном чтении неоткрытого устройства.

CI использует закреплённый **windows-2022** runner, MSVC target, frontend export перед компиляцией Tauri, полный `npm run qa`, явные fixtures, release build и запуск `.exe` probe. CMake закреплён на 3.31.6: включённый SDL объявляет CMake 3.0 compatibility, которую CMake 4 больше не принимает без policy override. Системные SDL DLL и отдельная установка SDL не требуются: `dumpbin /DEPENDENTS` локального release PE подтвердил отсутствие `SDL2.dll`. MSVC runtime DLL (`VCRUNTIME140*`, UCRT) остаются обычными зависимостями; static-link здесь означает SDL, не весь CRT/Windows.

Первый CI run `40980035691` на `windows-latest` **FAILED** до fixtures: cmake-rs выбрал установленный `Visual Studio 18 2026`, generator которого отсутствует в CMake 3.31.6. Подтверждённая ошибка: `Could not create named generator Visual Studio 18 2026`. Исправлена пара runner/toolchain: windows-2022 + CMake 3.31.6; зависимость SDL и её исходники не подменены.

Второй CI run `40980838685` на windows-2022 собрал SDL и прошёл все 3 input fixtures, но общий QA дал **543 passed / 4 failed / 4 ignored**: существующий patch helper выбрал `C:/hostedtoolcache/windows/Python/3.12.10/x64/python3.exe`, и hash_file отклонил reparse-point alias с InvalidPath. Исправляется только среда нового workflow: физический `os.path.realpath(sys.executable)` задаётся через уже существующий `FUSION_TEST_PYTHON` (`transforms.rs:1076-1093`). Workflow проверяет отсутствие reparse points у самого exe и всех предков. Защита путей/production-код/test helper не ослаблены.

Третий run: **[40981975222](https://github.com/blessdevhq/fusion-launcher/actions/runs/40981975222)**, source/workflow commit **d173e7b2de910f6566b7539bebdbb0b79701983c**, overall **FAILURE**. Windows-2022/CMake/physical-Python setup проходят. Typecheck, **142 Node tests**, static-check, build, **547 library Rust tests + 3 CLI tests**, fmt проходят; test suite использует обычную параллельность CI. Далее clippy под автоматически обновившимся **Rust 1.99.1** отклоняет **18** macro-generated `double_must_use` в существующих async_trait-модулях (adapters, integrity, resolver, torrent_engine), не в E input. Локальный Rust **1.95.0** с `-D warnings` проходит на том же исходном коде. Explicit fixture/release/probe steps этого CI пропущены из-за предыдущего неуспеха; release app probe **на CI ещё NOT TESTED**, несмотря на локальный PASS и фактически выполненные input-тесты в общей suite.

После сообщения об этом блокере и дальнейшего указания пользователя «продолжай» выбран минимальный вариант: только E-spike workflow закрепляет Rust **1.95.0**, как в локальном QA. Это не доказывает совместимость остальной базы с clippy 1.99.1: 18 предупреждений остаются задачей владельцев модулей при обновлении toolchain. `-D warnings`, все тесты и source-код сохранены; allow/ослабление lint и правки чужих модулей не применяются. Предыдущий doc-only handoff commit использовал skip-ci и не менял красный результат run 40981975222. Новый CI проверяется отдельно, до его завершения зелёный статус не заявляется.

## Измерения

Сравнение release app до/после использует одну рабочую директорию, один профиль/target/toolchain и один неизменный `out/`, без подмены на standalone harness. SHA-256 comparison всех **39** embedded frontend файлов подтвердил одинаковые байты до/после. Время baseline не является временем добавления SDL: в baseline были пересобраны существующие release-зависимости.

| Измерение | Выполненный результат |
| --- | --- |
| Release baseline без SDL, commit b08bc35 | **32 973 824 bytes**; build **597,334 с**, существующие release-зависимости частично пересобраны |
| Release с достижимым input-модулем и app probe | **36 027 904 bytes**; build **271,466 с**, остальной release cache прогрет, SDL собирался для этого профиля |
| Прирост `.exe` | **3 054 080 bytes = 2,9126 MiB = 9,2621%** |
| Native app `--input-spike-probe` | **exit 0**, SDL **2.26.4**, devices **[]**, handles_closed **true**, initialized flags **0** |
| `dumpbin /DEPENDENTS` release `.exe` | **PASS**, SDL2 DLL не импортируется |

Пустой список устройств честно отражает этот запуск: probe не доказывает hardware input и не auto-open-ит устройство. Read/open доказаны virtual fixtures внутри launcher library. Размер включает worker, диагностическую CLI-проводку и статически достижимый SDL, а не абстрактную стоимость только Rust wrapper. Чистое время всего приложения с нулевым Cargo cache локально не измерено. Чистый packaging/NSIS size delta также не измерен.

Baseline без SDL: `.tmp/input-baseline-size.json` и `.tmp/input-baseline-build.log`.
Сравнение: `.tmp/input-size-result.json`, `.tmp/input-size-comparison-context.json`; сборка/запуск: `.tmp/input-sdl-release.log`, `.tmp/input-sdl-release-size.json`, `.tmp/input-app-probe.json`, `.tmp/input-pe-imports.log`. Native timing HTML записан Cargo в shared target `cargo-timings/cargo-timing-20261030T003752987Z-987742b7e7b7cb8e.html`. CI artifact `native-input-windows-evidence` предназначен для JSON результатов и cargo timing HTML; его фактическая доступность проверяется после успешного run.

## Риски и NOT TESTED

### Локальный полный QA

`RUST_TEST_THREADS=4 CARGO_TARGET_DIR='F:/MVP/Fusion Launcher/src-tauri/target' npm run qa` — **PASS, exit 0**: typecheck, **142 Node tests**, static-check, production frontend build, **547 Rust library tests + 3 Packer CLI tests**, fmt и all-targets clippy `-D warnings`. Четыре исходных теста packaged-sidecar остаются ignored и не считаются прошедшими. Число test threads ограничено явно; этот результат не переименовывается в успешный прогон с обычной параллельностью. Лог `.tmp/input-qa-four-threads.log`.

Все 3 input-теста дополнительно выполнены в **10 последовательных запусках** уже собранного launcher test binary: в каждом 3 passed / 0 failed. Evidence `.tmp/input-fixture-repeat.json`, `.tmp/input-fixture-repeat.log`.

### Одновременное чтение и shell

**CONFIRMED по исходникам**: `src/hooks/useGamepad.ts:33-75` подписывает только `window.keydown`; browser Gamepad API polling отсутствует. Поиск по `src/` не нашёл `navigator.getGamepads`, gamepadconnected/disconnected или собственного dispatchEvent для GamepadA. Строки `GamepadA/B/Y` в `src/lib/focusManager.ts:20-24` — action aliases, не reader и не генератор событий. `Dashboard.tsx:734` подключает этот hook. В исходном грязном worktree hook прочитан отдельно: это тот же keyboard-only путь. Оба frontend файла в E не изменены.

**Следствие текущей реализации**: новый SDL worker не генерирует keydown и не отправляет input IPC; дублирование shell activation этим модулем не введено. Фактического «одновременного чтения того же GameController браузером и SDL» в текущем коде нет — утверждать проверенный конфликт или проверенную совместимость нельзя.

**STRONG INFERENCE для будущей интеграции**: если F начнёт маршрутизировать native input в shell или появится browser reader, один физический press может попасть в два потребителя. До этого F должен ввести явный capture owner, не маршрутизировать захватываемые inputs в shell и ограничить modal focus scope. Сейчас `collectFocusTargets` (`useGamepad.ts:78-94`) ищет targets по всему document, hook не проверяет `defaultPrevented`/input-session ownership; один лишь `preventDefault` в будущем capture handler не доказывает подавление shell. Внешний keyboard-emulation слой контроллера также может давать keydown независимо от SDL. Это риски, не воспроизведённые аппаратные конфликты; E не исправляет чужую навигацию.

Нативные virtual joysticks SDL не публикуются как Windows HID/XInput для WebView2. Нужен отдельный физический smoke SDL + shell/WebView2 на той же кнопке, с focus/background и при наличии внешних remappers. Успешный virtual fixture этот smoke не заменяет.

### Ошибки проверок, которые не скрыты

Первый локальный `npm run qa` дал **545 passed / 2 failed / 4 ignored** в Rust: `cancelling_job_waits_for_actual_descendants_and_drops_all_base_locks` и `dropping_caller_future_cancels_tree_without_dropping_lease_early` остановились на `supervisor_tests.rs:500`, `descendant failed to publish pid` (20-секундное ожидание). Оба точных теста затем отдельно прошли (4,32 с и 2,91 с). Повторный полный прогон дал **546 passed / 1 failed / 4 ignored** на первом из них. Все 3 SDL-теста прошли в обоих полных прогонах. Причина различия isolated/parallel **UNCONFIRMED**; supervisor не изменён, timeout не увеличен, тесты не отключены. Его fixture копирует и запускает собственный test exe (`supervisor_tests.rs:238-255`), поэтому длительность дочернего старта зависит и от среды. Это наблюдение, не доказательство конкретной причины. Логи `.tmp/input-qa.log`, `.tmp/input-qa-retry1.log`, `.tmp/input-supervisor-repro1.log`, `.tmp/input-supervisor-repro2.log`.

Дополнительное сравнение двух уже собранных test exe с обычной параллельностью: baseline **544 passed / 0 failed / 4 ignored**, SDL-вариант **546 passed / 1 failed / 4 ignored** — на существующей проверке очистки `.fusion-patch-*` (`transforms.rs:1173`). Набор test names отличается только тремя input-тестами. Один такой differential run не устанавливает причину; cleanup-flake уже наблюдался до E в wave-zero handoff. Исходники transforms/supervisor в E не изменены. Evidence `.tmp/input-differential.json`, `.tmp/input-differential-baseline.log`, `.tmp/input-differential-sdl.log`. Первая попытка захвата этого диагностического вывода потеряла результат из-за не-UTF8 Windows stderr и не считается проверкой; повтор сохраняет исходные bytes.

Независимый read-only review E: **PASS**, security_concerns и logic_errors пусты. До production нужны fault-injection проверки init failure/timeout/panic, политика hung worker и API deselection/reselection. Для single-selection спайка это отмечено как неблокирующие ограничения; общие IPC всё ещё action_unavailable.

- Физический контроллер, USB/Bluetooth, driver-specific mapping и одновременное чтение реального устройства SDL/WebView2: **NOT TESTED**. Virtual SDL не является виртуальным XInput/HID устройством для Chromium и не доказывает такое взаимодействие.
- ОС-level освобождение реального hardware перед Eden: **NOT TESTED**. Автоматическое доказательство относится к собственным SDL references и worker.
- F должен использовать capture/focus ownership до подключения input к shell; E не изменяет чужие frontend модули.
- Без production IPC/state store спайк не является готовым E-сервисом v1. Calibration, сохранение bindings и ParamPackage намеренно отсутствуют.
- Frontend/i18n/CSS не менялись; `mvp:visual` не заменяет аппаратную проверку и не запускается для этого backend-only delta.
- Общий локальный node_modules переиспользован через junction; чистый npm install проверяется отдельным CI runner, а не заявляется локально.
