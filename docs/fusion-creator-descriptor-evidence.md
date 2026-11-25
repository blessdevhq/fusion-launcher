# Fusion Creator: descriptor evidence и расхождения

Наблюдение: **2026-10-31**. База `creator-wave1-base` / `0ce4598`, worktree
`F:/MVP/Fusion-Creator-Contracts`, ветка `creator/contracts`.
Решение: заморозить минимальное декларативное описание **Eden 0.2.1 и обычной игры
SuperTuxKart 1.5**, не подменяя JSON contract работающим Creator.
Нормативный формат — [contracts](fusion-creator-contracts-v1.md#programdescriptor-10--нормативная-заморозка).

## SuperTuxKart: portable архив действительно существует

**CONFIRMED / primary release + реальные байты:** официальный `SuperTuxKart-1.5-win.zip` —
installer-less Windows архив, не `.exe` installer, переименованный в ZIP. Сайт выделяет
его отдельно от installers; GitHub release опубликован **2025-10-20T16:48:01Z**.
Это дата события, а не дата нынешней проверки.[1][4]

| Проверка | Реальное значение |
| --- | --- |
| Release/tag | `1.5`; source commit `1fb491f507216c5d181ccd85f29ff08eca003827` |
| ZIP size | `922430727` bytes |
| Локальный SHA-256, совпавший с release API | `9df7e2d67e8562127a3bb633030f1bd4ee77fa9e7f0ae110897473025af8acdc` |
| Transport BLAKE3, все байты | `ea5830dc60aad988fa045259220a88e71a121c7bb3359fbee00e59c413114ad6` |
| ZIP directory entries | `6098` |
| Нормализованные файлы / их bytes | `5763` / `1586076164` |
| Нормализованный CAS-tree BLAKE3 | `9f1b886deb979f6b6c2d59b4d6ae820faf5f900b5877066e8798300e57676bf9` |

Локальная проверка: архив streamed to disk, SHA-256/BLAKE3 streamed; каждый ZIP member прочитан
чанками 1 MiB с проверкой CRC. Удалён только точный root `SuperTuxKart-1.5-win`; пути отсортированы,
inventory identity вычислен по **существующему** `cas.rs:328–353`:
`BLAKE3("fusion-source-cas-tree-v1" + NUL + compact UTF-8 JSON [path,blake3,size records])`.
Это воспроизведение алгоритма на настоящих bytes, **не** вызов будущего Creator normalizer.
[Машиночитаемый результат](evidence/supertuxkart-1.5-inspection.json) содержит digest и sample file hashes.
Архив/временные Python зависимости в Git не включены. Пакеты проекта и lock-файлы не менялись.

Фактическая структура **опубликованного архива**, не догадка по packaging workflow:[5]

```text
SuperTuxKart-1.5-win/
  run_game.bat
  stk-code/
    build-x86_64/bin/supertuxkart.exe
    build-x86_64/bin/libcurl.dll
    build-i686/bin/supertuxkart.exe
    build-aarch64/bin/supertuxkart.exe
    build-armv7/bin/supertuxkart.exe
    data/stk_config.xml
    data/supertuxkart.1.5
    data/{karts,tracks,music,sfx,textures,shaders,gui,...}/
```

**CONFIRMED:** batch выбирает архитектуру и делает `Pushd` в соответствующий `bin`, затем
запускает `supertuxkart.exe`; не настраивает portable savedir и не передаёт `%*`.
Для x64 descriptor берёт настоящий exe и тот же cwd напрямую. Fusion не исполняет `.bat`.[23][5]

**CONTRADICTED / проверка против исходной гипотезы:** pinned packaging workflow описывает
отдельный `stk-assets`, но реальный ZIP его не содержит: assets находятся в `stk-code/data`.
Descriptor обязан следовать проверенному архиву, а не предположению по build script.[24][5]

### Пользовательские файлы не находятся в ZIP

**CONFIRMED / source, NOT runtime observation:** стандартный Windows root —
`%APPDATA%/supertuxkart/config-0.10/`, **не** `config-1.5`. Если существующий/создаваемый
`SUPERTUXKART_SAVEDIR=B` принят, config root становится `B/config-0.10/` без дополнительного
`supertuxkart`. Непригодный override может привести к fallback; host обязан заранее создать,
проверить и изолировать свои roots, а не считать переменную fail-closed sandbox.[6]

| Файл/путь при `B=workspace/userdata` | Назначение |
| --- | --- |
| `userdata/config-0.10/config.xml` | Settings XML `<stkconfig version="8">`; **не** bundled `data/stk_config.xml` |
| `userdata/config-0.10/players.xml` | Профили и progress; могут содержать приватное session state |
| `userdata/config-0.10/input.xml` | Собственные назначения приложения; не Fusion SDL bindings |
| `userdata/config-0.10/highscore.xml` | Именно singular `highscore.xml`; отдельный HIGHSCOREDIR override |
| `userdata/config-0.10/{screenshots,replay,grandprix,cached-textures}/` | Screenshots, replay, пользовательский GP, cache |
| `userdata/addons/` | Addons: приложение использует `config-root/../addons`, sibling config root |

Config XML/version подтверждены serializer; players/profile paths и saved-token — отдельными
source files; высокие результаты — `highscore_manager.cpp`. Эти данные нельзя публиковать как
«чистый template» пользователя.[8][20][21]

`SUPERTUXKART_HIGHSCOREDIR` задаётся в fixture явно на `userdata/config-0.10`.
`SUPERTUXKART_DATADIR` указывает на `app/stk-code`: source **добавляет `/data/`**,
поэтому передавать `app/stk-code/data` неверно. Asset flags не заменяют savedir.[6][22]

Простые проверенные адреса в `config.xml`: `/stkconfig/Video/@fullscreen`, `@show_fps`,
`@real_width`, `@real_height`, `/stkconfig/Audio/@music_on`, `@sfx_on` и
`/stkconfig/GFX/@swap-interval-vsync`. В source `width/height` помечены «should not be edited»;
для resolution нужны `real_width/real_height` или проверенный `--screensize=WxH`.
Fixture намеренно ограничивается fullscreen/show_fps/music booleans, без нелинейного audio slider.
CLI подтверждает `--windowed`, `--fullscreen`, `--screensize`; выдуманного `--portable` или
`--user-config-dir` в descriptor нет.[7][9]

**CONTRADICTED:** installer-less ZIP сам по себе не означает self-contained user state.
Официальный batch оставляет default APPDATA discovery. **UNCONFIRMED:** реальный runtime footprint,
первый запуск, созданный config, аппаратные effective defaults и Unicode-path smoke не проверялись.[23][6]

## Eden 0.2.1: данные против capability-кода

**CONFIRMED:** canonical upstream release `v0.2.1` опубликован **2026-06-01**;
source tag закреплён на `58c1e20ee58efa3900ba616207d460886214480b`. Release прямо говорит,
что это SDL2, не SDL3. `0.2.1` здесь версия программы, не заявление о latest и не версия adapter API.[10]

**CONFIRMED / pinned source:** Windows portable marker — каталог `user` рядом с exe;
иначе используется roaming AppData `eden`. Config root — `user/config`, default Qt config name —
`qt-config`, итоговый файл `qt-config.ini`. CLI frontend имеет отдельный `sdl2-config.ini`:
GUI/CLI варианты нельзя смешивать.[11][13][19]

Пример выбрал нормализованный directory root с `eden.exe`, mount `eden`; он **не утверждает**
конкретный top-level каталог любого upstream `.7z`. `[Renderer] backend` — простой integer
(0 OpenGL_GLSL, 1 Vulkan). Обязательный companion `backend\default=false` объявлен **данными**:
source иначе читает default и игнорирует выбранное значение. Ini codec сохраняет literal backslash
в key; это не путь и не строковый скрипт.[14][18][13]

**CONFIRMED:** ParamPackage имеет separators `:` и `,`, escape `$0/$1/$2`, empty placeholder
`[empty]`; это специализированная сериализация, не generic INI scalar и не source-defined script.
Она остаётся optional host capability `eden_param_package_v1` вместе с Qt/input translation.[12]

Eden fixture **schema-only**: no resolved runtime artifact, no full tested input mapping, нет games,
keys или firmware. `managed.redistribution_allowed=false` не выдаёт permission. Реальный release,
inventory и ручная/GUI проверка Eden **NOT TESTED**. Таймаут исследовательского агента не выдан
за PASS: родитель прочитал сохранённые pinned sources и release/tag evidence непосредственно.

## Forza Horizon 2 + Azimuth — только на бумаге

**CONFIRMED / что заявляет upstream:** Almost There Games публикует Azimuth на itch.io;
страница перечисляет Xbox One **Forza Horizon 2 `1.0.0.64`**, Storm Island `1.0.0.36`.
Это не Xbox 360 версия. Версия runtime на исследованной странице/post — Azimuth `1.0.4`;
это отдельная сущность от game/expansion versions. Полная compatibility matrix, обязательность
expansion для base-game profile и byte identity из одной строки не следуют.[15][17]

**CONFIRMED / права:** автор: «Azimuth is free to download and use. It is not free to take:
it is all rights reserved, and nobody may redistribute it, modify it or rename it without
our permission». Для этого evidence set выбирается **`user_provided`**, не одобренный `managed`.
Managed возможен только после отдельного разрешения на конкретные публикуемые bytes/версию;
бесплатность, похожий GitHub repo/mod license, metadata подпись или boolean его не заменяют.[16]

**CONFIRMED / пользовательский input:** upstream требует собственные game files и не включает
их в проект. Структурный hint — папка с `era.vbi`, `Mount/`, `EmbeddedXvd/`.
Это hint классификации, не доказательство build, происхождения или целостности.[15]

| Аспект | Выразимость в замороженном формате / ограничение |
| --- | --- |
| Program/runtime identity | Да: identity Azimuth + точная version, отдельно от executor API. Публичные имя/version не заменяют проверенный binary inventory. |
| Game `1.0.0.64`, expansion `1.0.0.36` | Opaque game metadata и отдельные pinned components можно назвать; структурированное required-game/content-version отношение отсутствует. Не объявлять game_version compatibility resolver. |
| Пользовательская игра | Origin user_provided + requirement_id + точный inventory выражает проверенный input. Реальные hashes не установлены: готовый descriptor не создавался. Не выдавать era.vbi/имя папки за хеши. |
| Сам Azimuth | User_provided до отдельного permission review; runtime и game origins независимы. Managed approval не выводится из code/mod license. |
| Патчи | Projection rule выбирает готовые проверенные bytes, **не** применяет binary patch. Upstream update advice «extract over your Azimuth folder» — runtime replacement, не доказательство game patch и не безопасный transactional plan. |
| Launch/config/persistence | Публичный UI workflow не является проверенным CLI/config API. Entrypoint/layout/user-state paths не додумывались и не проверялись запуском. |

**Gaps, не добавлять в v1 ради этого кейса:**

1. Xbox Title ID, title-update/package/content IDs, edition/region и structured required-version /
   expansion dependency matrix. Неизвестны реальные expected byte pins.
2. Required game/runtime patch recipes: format, exact target, pre/post hashes, ordering и rollback.
   Не установлен даже конкретный необходимый patch; существующий legacy `PatchSpec` сам по себе
   не даёт descriptor semantics и не означает подтверждённый patch для Azimuth.
3. Azimuth-specific automation/CLI/config migration/input translation/protocol и требуемые
   hardware/runtime preflight rules. Это не проверено, не реализуется в данной задаче.
4. Permission evidence/workflow для перехода user_provided → managed. Не ослаблять gates и не
   присваивать выдуманные inventory hashes ради schema-valid «реального» примера.

Локальный live-showcase manifest прочитан отдельно в оригинальном worktree:
`live-source/pages-dist/manifests/forza-horizon-2-showcase.json` содержит `runtime/emulator.exe`,
`core.dll`, `display.toml`, synthetic addons `addon-60fps`/`addon-crash-test`, `launch: null`.
По условию задачи это **synthetic fixture**, не настоящие game/Azimuth bytes, не доказательство
их совместимости, permissions, performance или успешного реального цикла. В эти файлы не писали.
Исключённый пользователем другой проект не оценивался и не проектировался.

## Расхождения с уже написанным и handoff

| Код/документ базы | Факт чтения / расхождение с frozen descriptor | Владелец / действие |
| --- | --- | --- |
| `src/types/fusionCreator.ts`; `fusion_creator/mod.rs:42–55` | CreatorProject 1.0 строгий, не имеет ProgramDescriptor; program identity/version нельзя прятать в AdapterReference. Новый CreatorProgramProject 1.1 здесь только schema/document. | C+F согласованно добавляют Rust/Zod/storage/migration, не ломая 1.0. |
| `fusion_source/creator_schema.rs` | CreatorManifest содержит adapter/components/profiles/rules, не новую ProgramCreatorManifest alternative. Нет application_defaults controls variant или profile settings. | A+C+F: typed variant и canonical/hash preservation, затем B/D consumers. |
| `fusion_source/schema.rs:156–165,188–205` | RuntimeSpec args/env строковые; PersistenceSpec только saves_paths; нет descriptor typed argv/cwd/file-vs-directory policy. | A/B/D: общий compiler/validator, без program-name branches; не использовать untrusted literal templates. |
| `fusion_source/config_templates.rs:14–36,77–110` | Legacy text renderer: ini/json/toml/cfg, не typed XML attrs и key-preserving profile writer. Нельзя назвать существующий template renderer поддержкой SuperTuxKart. | D+A: bounded INI/XML codec и atomic private candidate writes. |
| `fusion_source/persistence.rs:14–62` | При отсутствующем persistent path создаётся directory; для впервые отсутствующего players.xml/input.xml нужна явная file semantics. | B/D: будущая generic persistence implementation с регрессией; здесь production не меняется. |
| `fusion_creator/projection.rs` | Resolver/NormalizeDirectory/PrepareCandidate — function types; descriptor normalization/projection ещё не исполняются. | A/B/D: одна реализация, не второй resolver. |
| `commands/fusion_creator.rs` и install gates | Production IPC возвращает action_unavailable; 1.1 execution/profile gates закрыты. | C/A/B/D/G: снять только с реальной общей реализацией и поведенческими тестами. Schema freeze не снимает gate. |
| `docs/schemas/README.md` до этой задачи | «combined QA pending» устарело относительно опубликованного base; новые formats не являются зеркалом старых runtime DTO. | Документ исправлен; новая JSON-only граница названа явно. |
| Program hypotheses | Portable ZIP ≠ private portable state; реальные STK data paths ≠ layout build workflow; Eden backend без companion default flag недостаточен. | Исправлено в контракте/fixtures на основе artifact/source, без app-specific production code. |

**Блокеры исполнения остаются открытыми**, а не исправляются в contract-freeze scope.
Завершение этой задачи означает frozen documents/schema, не завершение реализации перечисленных потоков.
Общие Cargo/package/lock/IPC/i18n файлы не изменены. i18n namespaces и остальные владельцы
остаются как в contracts; ProgramDescriptor carrier изменения A/C/F согласуют перед implementation.

## Проверки и NOT TESTED

- RED: focused Node test падал на отсутствующем ProgramDescriptor; второй tracer — на отсутствующем
  CreatorProgramProject. Оба затем прошли; legacy fixtures продолжили проходить.
- `node --experimental-strip-types --test src/types/fusionCreatorJsonSchema.node-test.ts`:
  **PASS, 15 tests**, включая negative paths, selectors, list bounds, private fields и legacy rejection.
- Python `jsonschema.Draft202012Validator.check_schema` + два program fixtures: **PASS**.
- Полный `RUST_TEST_THREADS=4 npm run qa` в Windows вне sandbox: **PASS / exit 0**:
  typecheck, **151 Node tests**, static-check, Next build/export, **550 Rust library tests +
  3 CLI tests**, fmt, clippy `--all-targets -- -D warnings`; **4 прежних ignored**.
  В том числе прошёл `creator_harness_tests::native_desktop_install_launch_update_rollback_preserves_latest_save`:
  это реальный lifecycle **тестового H-приложения**, не runtime acceptance STK/Eden.
- Первый завершённый полный QA: **FAIL / exit 101**, Rust **546 passed / 4 failed / 4 ignored**.
  Все четыре patch-adapter failures — `transforms.rs:1124`, отказ при hash_file Python executable:
  `Lock directory: Filesystem entry is a reparse point`. Локальный `FUSION_TEST_PYTHON` был uv alias
  `cpython-3.11-windows-x86_64-none`; `Path.resolve(strict=True)` выявил физический каталог
  `cpython-3.11.15-windows-x86_64-none`. Исправлено **только локальное окружение runner**,
  не код/защита/таймауты тестов. После этого весь QA выполнен заново и прошёл без исключений
  для новых failures. Initial FAIL сохранён, не переименован в flake/PASS.
  Предшествующие background attempts не дали полного результата и в PASS не засчитываются.
- Окружение: VS2022 BuildTools Developer Prompt, bundled CMake, `RUST_TEST_THREADS=4`,
  `CARGO_BUILD_JOBS=4`, offline Cargo; общий target cache `F:/MVP/Fusion Launcher/src-tauri/target`,
  существующие Node dependencies. Это не проверка чистой установки зависимостей.
  Логи: `.tmp/descriptor/qa-verified.log` (FAIL), `.tmp/descriptor/qa-physical-python.log` (PASS).
  [Машиночитаемый итог с SHA-256 логов](evidence/creator-descriptor-verification.json).
- Independent read-only review: **PASS**, security/logic blockers отсутствуют; conforming validator
  отдельно проверил обе fixtures. Рекомендации: заполнить Sources (выполнено), будущие semantic-negative
  host tests по codec/seed/address/profile completeness/ownership (остаются задачей реализации).
- `mvp:visual`: **NOT TESTED**, UI/CSS/locale не менялись.
- Реальные install/launch/update/rollback/export через Creator для STK/Eden: **NOT TESTED**,
  пока общий executor не реализован. SHA/CRC/source/schema verification не подменяют этот отчёт.
- Реальная Forza/Azimuth связка: **NOT TESTED**, только paper analysis; payload не скачивался,
  собственные пользовательские game files, configs и keys не читались.

## Sources

[1] https://supertuxkart.net/Download.html
[4] https://api.github.com/repos/supertuxkart/stk-code/releases/tags/1.5
[5] https://github.com/supertuxkart/stk-code/releases/download/1.5/SuperTuxKart-1.5-win.zip
[6] https://raw.githubusercontent.com/supertuxkart/stk-code/1fb491f507216c5d181ccd85f29ff08eca003827/src/io/file_manager.cpp
[7] https://raw.githubusercontent.com/supertuxkart/stk-code/1fb491f507216c5d181ccd85f29ff08eca003827/src/config/user_config.hpp
[8] https://raw.githubusercontent.com/supertuxkart/stk-code/1fb491f507216c5d181ccd85f29ff08eca003827/src/config/user_config.cpp
[9] https://raw.githubusercontent.com/supertuxkart/stk-code/1fb491f507216c5d181ccd85f29ff08eca003827/src/main.cpp
[10] https://git.eden-emu.dev/api/v1/repos/eden-emu/eden/releases/tags/v0.2.1
[11] https://git.eden-emu.dev/eden-emu/eden/raw/commit/58c1e20ee58efa3900ba616207d460886214480b/src/common/fs/path_util.cpp
[12] https://git.eden-emu.dev/eden-emu/eden/raw/commit/58c1e20ee58efa3900ba616207d460886214480b/src/common/param_package.cpp
[13] https://git.eden-emu.dev/eden-emu/eden/raw/commit/58c1e20ee58efa3900ba616207d460886214480b/src/frontend_common/config.cpp
[14] https://git.eden-emu.dev/eden-emu/eden/raw/commit/58c1e20ee58efa3900ba616207d460886214480b/src/common/settings.h
[15] https://almost-there-games.itch.io/azimuth
[16] https://almost-there-games.itch.io/azimuth/devlog/1714255/0100-progress-and-a-note-about-copies-of-azimuth-floating-around
[17] https://almost-there-games.itch.io/azimuth/devlog/1738011/update-104
[18] https://git.eden-emu.dev/eden-emu/eden/raw/commit/58c1e20ee58efa3900ba616207d460886214480b/src/common/settings_enums.h
[19] https://git.eden-emu.dev/eden-emu/eden/raw/commit/58c1e20ee58efa3900ba616207d460886214480b/src/qt_common/config/qt_config.h
[20] https://raw.githubusercontent.com/supertuxkart/stk-code/1fb491f507216c5d181ccd85f29ff08eca003827/src/config/player_manager.cpp
[21] https://raw.githubusercontent.com/supertuxkart/stk-code/1fb491f507216c5d181ccd85f29ff08eca003827/src/config/player_profile.cpp
[22] https://raw.githubusercontent.com/supertuxkart/stk-code/1fb491f507216c5d181ccd85f29ff08eca003827/src/race/highscore_manager.cpp
[23] https://raw.githubusercontent.com/supertuxkart/stk-code/1fb491f507216c5d181ccd85f29ff08eca003827/tools/run_game.bat
[24] https://raw.githubusercontent.com/supertuxkart/stk-code/1fb491f507216c5d181ccd85f29ff08eca003827/.github/workflows/windows.yml
