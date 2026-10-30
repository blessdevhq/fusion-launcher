# Поток E: SDL внутри приложения — отчёт интегратору

Статус: **проверки выполняются; готовность пока не объявлена**.

## Область изменений

- Worktree `F:/MVP/Fusion-Creator-Input-Native`, ветка `creator/input-native`.
- База: `b08bc35`, фиксирующая уже принятые wave-zero контракты поверх E/H foundation. Изменения предыдущего интегратора сохранены отдельным prerequisite commit, а не переписаны в ходе E.
- Новый модуль `src-tauri/src/input/`, **не отдельный crate**. Предыдущий `tools/creator-input-spike` — исторический эксперимент; доказательством выполнения этого задания служат только новые тесты приложения и native app probe.
- Единственное добавление Cargo.toml: Windows-зависимость `sdl2 = { version = "=0.38.0", features = ["bundled", "static-link"] }`. Cargo.lock получает только её транзитивное замыкание и необходимые уточнения существующих ссылок version-compare.
- Общий `lib.rs` изменён только добавлением Windows-модуля и явно запрашиваемого headless probe. Обычный запуск не стартует SDL и не открывает контроллер.
- Новый `.github/workflows/input-spike-windows.yml`. Существующие CI/release workflows не изменены.
- Нет UI, калибровки, Eden ParamPackage, новых production IPC или замены DTO замороженного Creator/input контракта.

## Реальный worker и API спайка

`InputWorker::start → devices → open(instance_id) → read → stop`.

Один именованный worker владеет context, subsystem, event pump и максимум одним GameController. Повторный одновременный start отклоняется. Объекты SDL и указатели никогда не выходят из worker; через канал идут только owned значения. Перечисление не открывает физические устройства. Open заново разрешает ephemeral instance ID в актуальный device index и проверяет instance после открытия; неизвестная SDL mapping — ошибка, не угадывание раскладки.

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

CI использует Windows runner, MSVC target, frontend export перед компиляцией Tauri, полный `npm run qa`, явные fixtures, release build и запуск `.exe` probe. CMake закреплён на 3.31.6: включённый SDL объявляет CMake 3.0 compatibility, которую CMake 4 больше не принимает без policy override. Системные SDL DLL и отдельная установка SDL не требуются; импорт-таблица итогового PE будет проверена отдельно.

## Измерения

Сравнение release app до/после использует одну рабочую директорию, один профиль/target/toolchain и один неизменный `out/`, без подмены на standalone harness. Размеры, времена и import-table evidence дополняются только по завершённым командам. Время baseline не является временем добавления SDL: в baseline были пересобраны существующие release-зависимости.

Baseline без SDL: `.tmp/input-baseline-size.json` и `.tmp/input-baseline-build.log`.
Новые fixture/build/probe/QA evidence хранятся в `.tmp/input-*`. CI artifact `native-input-windows-evidence` содержит JSON результатов и cargo timing HTML.

## Риски и NOT TESTED

- Физический контроллер, USB/Bluetooth, driver-specific mapping и одновременное чтение реального устройства SDL/WebView2: **NOT TESTED**. Virtual SDL не является виртуальным XInput/HID устройством для Chromium и не доказывает такое взаимодействие.
- ОС-level освобождение реального hardware перед Eden: **NOT TESTED**. Автоматическое доказательство относится к собственным SDL references и worker.
- F должен использовать capture/focus ownership до подключения input к shell; E не изменяет чужие frontend модули.
- Без production IPC/state store спайк не является готовым E-сервисом v1. Calibration, сохранение bindings и ParamPackage намеренно отсутствуют.
- Frontend/i18n/CSS не менялись; `mvp:visual` не заменяет аппаратную проверку и не запускается для этого backend-only delta.
- Общий локальный node_modules переиспользован через junction; чистый npm install проверяется отдельным CI runner, а не заявляется локально.
