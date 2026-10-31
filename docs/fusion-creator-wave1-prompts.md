# Стартовые промпты волны 1

Перед запуском проверить удалённый `creator-wave1-base` и записать **peeled commit SHA**
в отчёт слота. Все три слота получают один commit; старые `creator-base` и dirty worktree
не являются базой. Контракты читать первыми; затем AGENTS.md, план и wave1-base report.

## Общие обязательные правила для трёх слотов

- Собственный git worktree и ветка `creator/<поток>` от `creator-wave1-base`; не трогать чужие worktree.
- Цель — общая система подключения portable-программ **данными**, без per-program правок
  Rust/TypeScript, IPC/platform registry и пересборки Fusion. Eden — первый прикладной профиль,
  не архитектурный special case. После него нужна вторая независимая реальная программа;
  синтетический fixture H не подменяет эту проверку.
- Контракты зафиксированы; изменения Rust/TS/JSON Schema/fixtures проходят владельца и интегратора.
  Cargo.toml/lock, package.json/lock и общую IPC-регистрацию меняет интегратор, только scoped additions.
  Единственный редактор `src/lib/i18n.ts` — F; передавать ключи в своём namespace, без новых пустых locale.
- Production не сообщает фиктивный успех; незавершённое действие остаётся `action_unavailable`.
  Большие загрузки/хеширование стримятся на диск. Не ослаблять пути, подписи, TOFU, executable approval.
- Тесты расширяют `crate::fusion_source::test_support::CreatorHarness` под `cfg(test)`.
  Не копировать installer/transport/конструктор и не создавать production fixture backdoors.
- `RUST_TEST_THREADS=4 npm run qa`; для UI дополнительно `npm run mvp:visual`.
  При внешнем `CARGO_TARGET_DIR` предварительно создать локальную ignored `src-tauri/target`.
  Использовать существующий `FUSION_TEST_PYTHON` с проверенным physical Python.
- Два исторически падавших теста:
  `fusion_source::supervisor::tests::native::partial_graph_startup_failure_stops_started_support_and_releases_lease`
  (sharing violation / Windows error 32) и
  `fusion_source::supervisor::tests::native::cancelling_job_waits_for_actual_descendants_and_drops_all_base_locks`
  (`descendant failed to publish pid`). Результаты серий 10 запусков и полный triage — в
  `docs/fusion-creator-wave1-base.md`; не считать standalone серию доказательством стабильности под нагрузкой.
- Если QA упал **только на одном** из этих тестов с тем же историческим assertion, перезапустить
  его **один раз** с полным именем, `--lib -- --exact --nocapture`. Сохранить оба исхода.
  Успешный retry обозначается **FLAKY**, не PASS первоначального QA. Повторный сбой, оба теста,
  другой assertion или любое другое падение — FAIL/диагностика, а не blanket-exemption.
  При изменении supervisor/launch/workspace/fixture пути сравнить с неизменённым tagged baseline;
  одно совпадение имени не доказывает, что это не регрессия слота.
- Отчёт: точный base SHA и commit слота, изменения/контракты/границы, команды и реальные результаты,
  FLAKY с обоими исходами, NOT TESTED, предложения интегратору по shared files. Не объявлять
  полный Creator v1 готовым из-за компиляции, DTO, моков или виртуального контроллера.

## Слот A+B — общий Source/Packer/installer/resolver

```text
База: тег creator-wave1-base. Контракты зафиксированы в docs/fusion-creator-contracts-v1.md (читай первым).

Роль: A+B, один совместный слот волны 1. Ветка creator/source-install, отдельный worktree.
Прочитай AGENTS.md, docs/fusion-creator-v1-plan.md (разделы A/B и универсальная архитектура),
docs/fusion-creator-wave1-base.md и общие правила этого документа.

Границы: fusion_source wire/canonical/evolution/Packer, переносимые URLs и происхождение слоёв;
GameOptions/PlanOptions/ExecutionPlan, workspace/snapshots/integrity/approval;
fusion_creator/projection.rs, общий resolver и candidate/normalization boundary.
C владеет project/jobs/storage, E — native input; их реализацию не менять.

Сделай из единого плана A и B:
- Доведи manifest 1.0/1.1 parsing/наследование/required capabilities, сохраняя 1.0 рецепты и hashes.
  В baseline часть reader/profile plumbing уже есть: сначала прочитай код, не объявляй это новым.
- managed/external/user_provided recipes, pinned external inventories, portable relative URLs
  и сохранение исходного Source у inherited layers, без утечки reference payload/путей.
- ОДИН чистый resolve_projection по замороженной сигнатуре для Packer и installer.
  Одинаковый path/hash совместим, разные хеши требуют explicit winner; dependencies не дают priority.
  Проверяй каждый допустимый профиль, case/path/file-parent collisions и реально поставляющий слой.
- Конечные build/control pairings, staging и применение только через update/candidate,
  integrity и executable approval от реального winning provider.
- Общая детерминированная streaming-нормализация автор/пользователь; candidate preparation
  не меняет active config, saves или private bindings. Политику hardlink/cross-volume не обходить копией.
- Общие program descriptor/entrypoint/profile/persistence механизмы не зависят от Eden и не требуют
  новых platform enums/IPC/program-specific Rust веток для очередной portable-программы.
  При необходимости изменения frozen DTO остановись на согласовании с интегратором, не дрейфуй молча.

Готово, когда: Packer и installer используют один resolver; неверные origin/hash/profile/conflict
блокируются до mutation; старый 1.0 работает; нет reference/private утечек; update/rollback/error/cancel
сохраняют активную генерацию и saves. Удали соответствующий unavailable gate только для полностью
реализованного и реально проверенного пути. Подключи свои тесты к общему H helper.
Проверки/FLAKY/отчёт — по общим правилам. Не реализуй C runner, Eden ParamPackage, UI или SDL worker.
```

## Слот C — backend Creator, storage и jobs

```text
База: тег creator-wave1-base. Контракты зафиксированы в docs/fusion-creator-contracts-v1.md (читай первым).

Роль: C, волна 1. Ветка creator/backend, отдельный worktree.
Прочитай AGENTS.md, docs/fusion-creator-v1-plan.md (раздел C и универсальная архитектура),
docs/fusion-creator-wave1-base.md и общие правила этого документа.

Границы: fusion_creator project/jobs/releases/reports/private bindings, Creator handlers в
commands/fusion_creator.rs, одна согласованная storage migration. Не менять A+B resolver/Packer/workspace
или E worker/input handlers; shared registrations/additions передавать интегратору.

Сделай:
- Реальное атомарное сохранение/загрузку переносимого CreatorProject и отдельные private bindings.
- Единую миграцию SQLite: сам проверь актуальную схему и возьми СЛЕДУЮЩИЙ СВОБОДНЫЙ номер,
  не полагайся на 5/6 из исторических отчётов. G использует твой API, не конкурирующую миграцию.
- Реестр jobs/releases/reports, pinned project revision, один writer на workspace,
  progress/cancel/recovery с jobId и persisted монотонным sequence.
- Пять согласованных durable resume boundaries; reuse только после перепроверки revision/input hashes/
  adapter version и атомарно сохранённого результата. После restart unfinished jobs -> interrupted.
- Закрытие UI не отменяет job; поздние события не оживляют terminal job; история immutable,
  прежний release/active installation не повреждается при cancel/error.
- Program-agnostic storage/runner: без Eden-specific schema/enum и специальных команд на каждую программу.
  G позже подключает build/verify/export handlers к твоему runner; не строить второй Packer/installer.
- Только реализованные команды становятся доступными. Отсутствующие D/E/G операции возвращают
  action_unavailable; не выдавать fake job/release/report как реальный результат.

Готово, когда: реальные project/job операции сохраняются и восстанавливаются; stale checkpoint
не применяется к новой ревизии; прерывание не портит прошлый выпуск; существующие legacy данные читаются.
Подключи собственные тесты к cfg(test) H helper. Проверки/FLAKY/отчёт — по общим правилам.
Не реализуй Source recipes/resolver, SDL, Eden ParamPackage, UI или export policy G.
```

## Слот E — продолжение native SDL-спайка

```text
База: тег creator-wave1-base. Контракты зафиксированы в docs/fusion-creator-contracts-v1.md (читай первым).

Роль: E, третий слот волны 1; продолжай существующий native-спайк, не начинай второй crate/worker.
Ветка creator/input-wave1, отдельный worktree. Прочитай AGENTS.md, docs/fusion-creator-v1-plan.md,
docs/fusion-creator-input-native-spike.md, docs/fusion-creator-wave1-base.md и общие правила документа.

Границы: src-tauri/src/input и согласованный input boundary fusion_creator/input.rs;
свои cfg(test) fixtures и input evidence. Cargo/lock/регистрацию IPC передавать интегратору;
Creator handler file остаётся у C. src/hooks/useGamepad.ts и focusManager пока только исследовать,
без UI/навигационных изменений в этом слоте.

Цель: продолжить уже существующий SDL0.38.0 bundled/static-link спайк внутри приложения.
Сначала прочитай worker/fixture/report: локальная и Windows CI сборки уже проверялись в истории,
но не приписывай этот исторический CI своему новому коммиту.
- Подтверди применение worker boundary/InputProvider contract без SDL context/handles в UI/Tauri State.
- Перечисление устройств, один GameController, buttons/hats/sticks/triggers, unknown mappings без угадывания;
  teardown/stop ACK только после реального закрытия и join. Timeout не означает successful release.
- Изучи и закрой подтверждённые spike gaps (hung stop/join, reconnect/disconnect, актуальность snapshots)
  минимальными тестируемыми изменениями. Если нет воспроизведения, записывай риск, не выдумывай фикс.
- Сохрани test-only virtual fixtures, отсутствие production backdoors, size/build/probe evidence,
  и различие SDL crate version/runtime native version. Input — capability для программ,
  не Eden-only special case и не обязательное условие для любой portable-программы.
- Взаимодействие с реальным physical устройством/WebView2/Eden не считать проверенным виртуальным SDL.
  Недоступные hardware сценарии явно NOT TESTED.

Не делай: Eden ParamPackage, UI, калибровку, program-specific registry, job/storage C.
Готово для этого слота: Windows build и реальные input fixture/probe результаты нового коммита,
обоснованные риски/ограничения и предложения wiring интегратору, без fake success на stop/IPC.
Проверки/FLAKY/отчёт — по общим правилам; продление полного input service идёт в следующую волну.
```
