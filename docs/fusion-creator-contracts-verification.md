# Проверка контрактного каркаса Creator

## Итог

**PASS для контрактной ветки `creator/contracts` от `creator-base` (`287dc37`).**
Это DTO/сигнатуры и явно недоступный IPC, не готовый Creator и не интеграция SDL/H.
Полный QA прошёл с **`RUST_TEST_THREADS=4`**. Прохождение с обычной неограниченной
конкурентностью не заявляется: первоначальный неудачный прогон сохранён ниже.

Среда: Windows, Rust **1.95.0**, Node **24.14.0**, npm **11.9.0**.
Зависимости Node доступны через доверенный локальный junction; чистый `npm ci` не проверялся.
Общий Cargo build cache не является общей SQLite/library/workspace или sandbox.

## Команды и результаты

| Команда | Фактический результат |
| --- | --- |
| `node --experimental-strip-types --test src/types/fusionCreator.node-test.ts src/types/fusionCreatorJsonSchema.node-test.ts src/types/fusionCreatorProfile.node-test.ts` | **15 passed, 0 failed** |
| `npm run typecheck` | **PASS** |
| `python .tmp/validate-creator-contracts.py` | **PASS**: conforming `jsonschema.Draft202012Validator` проверил metaschema, project/job/input roots, CreatorManifest extension, выбранную пару профилей, неизвестные/пропущенные поля |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib torrent::runtime_tests:: -- --nocapture` | **13 passed, 0 failed, 1 прежний ignored** после подготовки локальной target-папки |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib fusion_source::supervisor::tests::native:: -- --nocapture --test-threads=1` | **12 passed, 0 failed** |
| **`RUST_TEST_THREADS=4 npm run qa`** | **exit 0**: typecheck, **144 Node tests**, static-check, Next build/static export, **537 Rust library tests + 3 CLI tests**, fmt, clippy `--all-targets -- -D warnings`; **4 прежних ignored** |
| `cargo check --manifest-path src-tauri/Cargo.toml --locked --offline --lib` | **PASS**, обычная сборка библиотеки без тестовой конфигурации |
| `git diff --cached --check`, added-line security scan | **PASS**, credential/shell/eval/pickle/dynamic-SQL patterns не найдены |
| Независимое read-only review staged diff | **PASS**, security/logic blockers отсутствуют; замечание о missing verification report закрыто этим файлом |

Rust QA исполнил `commands_never_report_mock_success`: все зарегистрированные Creator/input
команды возвращают `ActionUnavailable` с точным именем, а не фиктивный успех.
`shared_wire_fixtures_round_trip_without_private_paths` проверил serde round-trip проекта,
выбранной пары, extension и событий, отклонение приватного поля.
Нулевые test targets main/local/doc-tests не считаются самостоятельным покрытием.

## Воспроизведение полного успешного QA

Из корня worktree в Windows Git Bash, с установленными зависимостями:

```sh
export CARGO_TARGET_DIR='F:/MVP/Fusion Launcher/src-tauri/target'
export FUSION_TEST_PYTHON="$(python -I -c 'import os,sys; print(os.path.realpath(sys.executable))')"
python -c 'from pathlib import Path; Path("src-tauri/target").mkdir(parents=True, exist_ok=True)'
RUST_TEST_THREADS=4 npm run qa
```

Внешний `CARGO_TARGET_DIR` опционален; выше точная использованная конфигурация.
Локальная `src-tauri/target` нужна существующим torrent fixtures, использующим
`env!("CARGO_MANIFEST_DIR")/target`, независимо от размещения build cache.
Python override — существующая настройка patch fixtures, не новая зависимость/обход path policy.
Ограничено лишь число test threads: ни один gate/assertion не отключён, новые ignored не добавлены.

## Исходный неудачный прогон — не удалён и не объявлен исправлением продукта

Первый `npm run qa` с внешним Cargo cache и обычной test concurrency:
**144 Node passed; 522 Rust passed, 15 failed, 4 ignored**.

- **13 torrent failures — подтверждённая ошибка подготовки worktree**: локальной
  `src-tauri/target` не было, `tempdir_in` в `torrent_runtime_tests.rs:240` вернул NotFound.
  Создание каталога устранило этот класс setup-ошибок; код torrent fixtures не изменён.
- Два native supervisor failures:
  `partial_graph_startup_failure_stops_started_support_and_releases_lease` (Windows error 32)
  и `cancelling_job_waits_for_actual_descendants_and_drops_all_base_locks` (descendant PID timeout).
  Отдельная native-группа прошла 12/12, затем полный QA с четырьмя threads прошёл.
  **Первопричина нестабильности обычной concurrency не установлена и не считается исправленной.**
  Supervisor/sidecar/тестовые таймауты/assertions не изменены.

Локальные логи (ignored `.tmp`, не production и не исходники коммита):
`contracts-red.log`, `contracts-profile-red.log`, `contracts-wire-green.log`,
`contracts-json-schema.log`, `contracts-qa.log`, `contracts-torrent-repro.log`,
`contracts-supervisor-repro.log`, `contracts-qa-four-threads.log`, `contracts-production-check.log`.
RED сначала обнаружил отсутствующий контрактный модуль, затем отсутствующий selection export;
GREEN исполнил завершённый набор тестов. Python validator script вспомогательный, в `.tmp`;
постоянные Zod/JSON Schema tests входят в обычный `npm test` без новых dependencies.

## Изменённые контракты и shared files

- Добавлены portable project/profiles/origins/rules, selection, Creator/input request/result/event,
  job/checkpoint/release/report DTO, JSON Schema и общие wire fixtures.
- Добавлены function types `ResolveProjection`, `NormalizeDirectory`, `PrepareCandidate`,
  `InputProvider` без реализации; никаких успешных resolver/job/build/export заглушек.
- Additive wiring: `lib.rs`, `commands/mod.rs`, `fusion_source/mod.rs`;
  в structured error enum добавлен `ActionUnavailable`.
- `Cargo.toml`, Cargo lock, package manifests/lock, i18n, migrations,
  legacy GameManifest/canonical/upcaster/Packer/plan/service/options/snapshots не изменены.
- [Контрактный документ](fusion-creator-contracts-v1.md) содержит все пять resume boundaries,
  единственных writers общих файлов, namespaces i18n и порядок изменения frozen DTO.

## NOT TESTED / вне этого коммита

- Реальные Creator project/job/build/verify/export/resume и SDL/input sessions: **не реализованы**,
  все команды явно недоступны. Нельзя трактовать DTO или успешный QA как работу этих функций.
- Full manifest 1.1 reader/install, применение профилей, алгоритм projection resolver,
  normalization/candidate, SQLite migration/recovery: реализация и тесты владельцев A–G впереди.
- Совместная SDL/H сборка на этой ветке, физический контроллер, Eden/gameplay, внешний HTTPS,
  cross-volume, cold/clean installs и CI именно этого коммита: **NOT TESTED**.
- `npm run mvp:visual`: **NOT RUN**, UI/CSS/i18n не менялись.
- Четыре существующих ignored Rust tests остаются ignored; их покрытие не заявляется.
