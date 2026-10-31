# Fusion Creator v1 — контрактный каркас волны 0

Авторитетный контракт объединённого исходного baseline Creator; Creator v1 ещё не реализован.
Исходный DTO-only коммит `4cf8971` ветки `creator/contracts` от `creator-base`
(`287dc370b891577c5c5ca27ac9126766daf89920`) имел только контрактный scope A–H.
Утверждения о его узких границах ниже относятся к истории этого коммита, а не к текущему
объединению `8fa5898` с `0165f71`, включающему SDL/H и native input worker.
**Combined QA этого объединения ещё не выполнен; green/frozen readiness не заявляется.**
Исторические результаты и NOT TESTED scope сохранены отдельно в
[fusion-creator-wave0-audit.md](fusion-creator-wave0-audit.md) и
[fusion-creator-wave0-handoff.md](fusion-creator-wave0-handoff.md); они не заменяют QA объединения.
Нормативная разбивка потоков: [план v1](fusion-creator-v1-plan.md).

## Объём и точки подключения

| Контракт | Rust | TypeScript / схема |
| --- | --- | --- |
| Профили, origin, portable projection rule, extension DTO | `src-tauri/src/fusion_source/creator_schema.rs` | `src/types/fusionCreator.ts` |
| Проект, выбранная пара профилей, задания, checkpoints, releases, reports, requests | `src-tauri/src/fusion_creator/mod.rs` | `src/types/fusionCreator.ts` |
| Чистая проекция и сигнатуры normalization/candidate | `src-tauri/src/fusion_creator/projection.rs` | Внутренний Rust API для A/B/D; не IPC |
| Input DTO и контракт `InputProvider` | `src-tauri/src/fusion_creator/input.rs` | `src/types/fusionCreator.ts` |
| Tauri-команды, пока только ошибки недоступности | `src-tauri/src/commands/fusion_creator.rs` | `CreatorApi`, `FusionInputApi` — интерфейсы без bridge-реализации |
| Общие wire-примеры | `fixtures/creator/contracts.json` | Те же fixtures для Node/Zod и Rust serde |
| JSON Schema draft 2020-12 | `docs/schemas/fusion-creator-contracts.schema.json` | [Правила применения и ограничения](schemas/README.md) |

Контрактный каркас `4cf8971` не читал каталогов, не создавал проект/задание/артефакт/выпуск,
не выбирал победителя и не применял профили; Packer, installer, snapshots и зависимости
не менялись именно в том DTO-only коммите. Объединённый исходный baseline уже включает
reader/canonical/storage, profile-field plumbing, native SDL worker и его зависимости.
Worker и SDL/H evidence не означают доступность production Creator/input IPC: перечисленные
ниже команды всё ещё недоступны, не создают job и не подтверждают освобождение handles.
Существующие transport, SQLite, Ed25519/TOFU остаются авторитетными; новые UI, локализации
и успешные production-моки этим контрактом не вводятся.

`CreatorManifest` теперь подключён как типизированное расширение `GameManifest.creator`
для manifest 1.1 и сохраняется в canonical/storage. DTO-only ограничение `4cf8971`
«не подключён к legacy reader» историческое и больше не описывает объединённый baseline.
`SourceIndex` wire schema остаётся `1.0`; metadata reader принимает GameManifest `1.0` и `1.1`.
Manifest `1.0` не может содержать `creator` или наследовать от `1.1`; `1.1` может наследовать
от `1.0`/`1.1`. Legacy Packer recipes по-прежнему выпускают `1.0`; отсутствующее расширение
и profile IDs не сериализуются, сохраняя legacy hashes/форму.
**Чтение не означает исполнение:** manifest `1.1` и non-null profile selection явно отклоняются
как `action_unavailable` до candidate mutation. A/B снимают execution gates только вместе
с полной реализацией и behavioral tests; resolution/build/export пока не реализованы.

## Формат данных и профили

- Переносимый `CreatorProject` имеет собственный `schema_version: "1.0"`, `project_id`, ревизию,
  title/source/game/version, pinned adapter, components, конечные build/control profiles и projection rules.
  Ключи переносимых документов — **snake_case**, оболочки IPC — **camelCase**.
- `BuildProfile { id, title, layer_ids, controls_profile_ids }` задаёт точный конечный набор слоёв
  и допустимые профили управления. Зависимости не устанавливают приоритет перезаписи файлов.
- `ControlsProfile { id, title, device, bindings }`: keyboard/mouse либо один SDL GameController;
  переносимые bindings логические. Физические GUID/path/serial/port, raw bindings и калибровка
  остаются в приватном локальном storage. Только D сериализует назначения в Eden ParamPackage.
- `CreatorProfileSelection { buildProfileId, controlsProfileId }` — выбранная точная пара;
  те же ID присутствуют в `CreatorReport` и параметрах `PrepareCandidateRequest`.
  Batch-job request содержит `buildProfileIds` и `controlsProfileIds`; runner обязан проверять
  только объявленные допустимые сочетания. Произвольное смешивание addons не разрешается.
- В объединённом baseline `GameOptions`, `PlanOptions`, `ExecutionPlan` содержат optional
  `buildProfileId` / `controlsProfileId`; snapshot хранит выбор в своём plan. Старые snapshots
  читаются без этих полей, отсутствующие IDs не сериализуются. Это plumbing, не применение:
  любой non-null выбранный ID вызывает `action_unavailable` до candidate mutation.
  Исходный `4cf8971` этих legacy-полей не менял; это только историческая граница. B отвечает
  за применение и execution tests без молчаливого игнорирования выбранных профилей.
- Каждый project component имеет `id`, `layer_id`, `origin`. В published extension — `layer_id` и origin,
  без приватных путей автора.
- Origins: `managed { artifact_ref, redistribution_allowed }`,
  `external { url, transport_blake3, inventory_blake3 }`,
  `user_provided { requirement_id, inventory_blake3, instructions }`.
  External требует закреплённых хешей и HTTPS без credentials. Флаг redistribution — данные,
  не выдача разрешения: G отдельно применяет allowlist и проверяет право экспорта.
- Reports раздельные: `structure_integrity`, `clean_install`, `observed_launch`, `manual_scenario`,
  `https_acquisition`; статус `passed|failed|not_tested`. Отчёт связан с immutable release/hash,
  ревизией проекта, adapter/version, выбранной парой профилей и environment.

Zod/JSON Schema проверяют форму и границы wire-значений. Rust serde DTO задают форму,
но не заменяют semantic validation у C/E/G: references, ranges, URL/path policy, hash/signature
и монотонность sequence не считаются проверенными простой десериализацией.

## Pure resolver: единая сигнатура для Packer и installer (B)

`ResolveProjection = for<'a> fn(ProjectionRequest<'a>) -> Result<ResolvedProjection, FusionError>`.
Будущая реализация называется `resolve_projection`; сейчас экспортирован **только тип функции**,
без исполняемого placeholder, `unimplemented!()`, IO, trait или альтернативного resolver.

Вход: явный режим `Legacy10|Explicit11`, выбранные `ProjectionLayer` (ID, mount path,
проверенный inventory `CasFile`) и правила `ProjectionRule { path, winning_layer }`.
`winning_layer` — portable-запись понятия **path → winningLayer**, не числовой приоритет слоя.
Выход: `ResolvedProjection.files`, детерминированная map пути в `ProjectionFile` с path,
winning layer, source-relative path, BLAKE3, size и equivalent layers.

Обязательная будущая семантика:

1. Только безопасные workspace-relative slash paths, Windows case-insensitive collision keys;
   traversal/absolute/UNC/device/ADS/reserved names и file/parent collisions — ошибки.
2. Один итоговый путь и одинаковый проверенный hash совместимы; сохраняются эквивалентные providers.
   Разный size при одинаковом hash — противоречивый inventory, не совместимость.
3. Разные хеши одного пути требуют явного авторского winning layer. Он обязан быть выбран
   и действительно поставлять путь. Неизвестный, исключённый или отсутствующий winner — ошибка.
4. Правила проверяются для каждого объявленного профиля. Dependencies не дают приоритета.
   Лишние/противоречивые решения не игнорируются молча.
5. Legacy 1.0 сохраняет прежнюю семантику. Выбор режима явный, без неявной миграции установок.
6. Resolve/validation выполняются до создания candidate; integrity и executable approval
   используют фактический provider. Результат resolver не даёт права писать вне inactive workspace.

`NormalizeDirectory` и `PrepareCandidate` тоже только function types. A/D используют один
streaming-normalizer; candidate preparation не меняет active config, shared saves или private bindings.
Алгоритмы и их тесты принадлежат A/B/D, не этой ветке.

## IPC и события

Каждая команда с аргументами принимает один Tauri-аргумент `{ request: DTO }`.
Списки имён зафиксированы в Rust/TS и сверяются тестом с реальной `generate_handler!` регистрацией.

| Команды | Request → response |
| --- | --- |
| `fusion_creator_project_list` | — → `CreatorProject[]` |
| `fusion_creator_project_open`, `fusion_creator_project_save` | project request / save request → `CreatorProject` |
| `fusion_creator_scan`, `fusion_creator_classify`, `fusion_creator_build`, `fusion_creator_verify`, `fusion_creator_export`, `fusion_creator_verify_https` | `CreatorJobStartRequest` → `CreatorJobReceipt` |
| `fusion_creator_job_get`, `fusion_creator_job_cancel` | `CreatorJobRequest` → `CreatorJob` |
| `fusion_creator_job_resume` | `CreatorJobRequest` → `CreatorJobReceipt` |
| `fusion_creator_release_list` | `CreatorProjectRequest` → `CreatorRelease[]` |
| `fusion_creator_report_get` | `CreatorReportRequest` → `CreatorReport` |
| `fusion_input_devices` | — → `InputDevice[]` |
| `fusion_input_session_start` | `InputSessionStartRequest` → `InputSessionReceipt` |
| `fusion_input_session_get` | `InputSessionRequest` → `InputState` |
| `fusion_input_session_stop`, `fusion_input_release_for_launch` | `InputSessionRequest` → `InputReleaseReceipt` |
| `fusion_input_bindings_save` | `InputBindingsSaveRequest` → `InputBindingsReceipt` |

**Сейчас все перечисленные production-команды возвращают `FusionError` с `code: "action_unavailable"`
и `details.command` — точным именем команды.** Даже list/devices не возвращают фиктивный пустой список.
Нет созданных jobId/sessionId и нет emitted progress. Typed result — будущий контракт, не успешная заглушка.

- `fusion-creator:job-progress`: `jobId`, `projectId`, `projectRevision`, `sequence`, kind/status/phase,
  completed/total units, nullable checkpoint/release/error.
- `fusion-input:state`: `sessionId`, `sequence`, nullable `deviceId`, status, logical buttons/axes,
  nullable error. Stick axes -1..1, triggers 0..1; без portable физической идентичности.
- `sequence`: положительный safe JSON integer, строго возрастающий **внутри job или session**.
  Job sequence сохраняется через restart/resume; reopen input получает новый session ID.
  Переполнение — ошибка; не сбрасывать/reuse sequence внутри прежнего ID.
- Consumer принимает только текущие ID/revision и большее sequence. Позднее событие не возвращает
  terminal job/session в running/active. Закрытие UI не отменяет job. Reopen сначала запрашивает состояние.
- Input state максимум 30 Hz; disconnect/terminal немедленно. Stop/release — только после worker ACK
  о фактическом закрытии handles. Таймаут — ошибка, не доказательство освобождения; launch заблокирован.

## Пять durable границ resume (C; G использует тот же runner)

| `CreatorCheckpointBoundary` | Минимальное атомарно сохранённое свидетельство |
| --- | --- |
| `scan_classification_saved` | Снимок результата **и сканирования, и классификации**, с ревизией/хешем классификации. Один scan недостаточен. |
| `artifact_committed` | Один полностью проверенный по expected bytes/inventory и атомарно опубликованный артефакт; `artifactId` обязателен. Отдельная граница для каждого артефакта. |
| `release_built` | Неизменяемый собранный выпуск с manifest и inventory артефактов, не промежуточный рецепт. |
| `verification_completed` | Полный immutable report для конкретного release/profile pair/environment. Не частичный запуск или acquisition. |
| `export_completed` | Полный allowlisted export атомарно опубликован, сохранён его inventory/hash. |

Каждый checkpoint содержит `checkpointId`, `jobId`, boundary, result BLAKE3 и stamp:
`projectRevision + inputHashes + adapter ID/version`. До reuse C/G повторно проверяют stamp
и сохранённый результат. Изменившийся/отсутствующий результат не применяется; частичный файл или
операция — не checkpoint. После restart незавершённые jobs становятся `interrupted`.
Workspace имеет одного writer. Cancel/error сохраняет предыдущий immutable release и active installation.
Эти правила зафиксированы как контракт; runner/storage/recovery не реализованы здесь.

## Владельцы общих файлов и namespace i18n

| Общий файл | Единственный writer / порядок |
| --- | --- |
| `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` (и root Cargo, если появится) | **Интегратор**. E/остальные присылают точные dependency additions; не перегенерируют lock параллельно. В исходном `4cf8971` изменений не было; SDL additions входят в объединённый baseline. |
| `package.json`, `package-lock.json` | **Интегратор**. Только согласованные additions, без конкурирующих install/перезаписи. Здесь без изменений. |
| `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs` | **Интегратор**: additive module/IPC wiring. C владеет Creator handlers/DTO, E — реализацией input; новые имена проходят сверку общего списка. |
| `src/lib/i18n.ts` | **F** — единственный редактор существующих en/ru словарей. Остальные передают ключи/текст; не заводят пустые параллельные locale-файлы. |
| Wire/canonical/Packer и `fusion_source/creator_schema.rs` | **A**; изменения profiles/projection согласуются с B и интегратором. |
| Plan/options/snapshots и `fusion_creator/projection.rs` | **B**, одна общая реализация resolver, вызываемая A/D/G. |
| `fusion_creator` project/jobs/reports, `commands/fusion_creator.rs`, migration | **C**. G подключает handlers к C runner/storage, не создаёт конкурирующую миграцию. |
| `fusion_creator/input.rs`, native input service | **E**, SDL worker ownership; input wire-изменения согласуются с C/F/D и интегратором. |
| `src/types/fusionCreator.ts`, JSON Schema и shared fixtures | Интегратор замораживает контракт; дальше **C + F**, с A/B/D/E по их DTO. Менять Rust/TS/schema/fixtures согласованно. |
| Общий integration/native harness | **H/H2**; unit-тесты реализаций остаются у A–G. SDL/H foundation уже включён; общий QA объединения ещё предстоит. |

| Поток | Namespace, которым владеет автор текста (вносит F) |
| --- | --- |
| A | `creator.source.*` |
| B | `creator.install.*` |
| C | `creator.project.*`, `creator.job.*` |
| D | `creator.adapter.eden.*` |
| E | `input.*` |
| F | `creator.ui.*`, `creator.common.*` |
| G | `creator.release.*`, `creator.verify.*` |
| H/H2 | нет production-ключей |

Изменение frozen DTO проходит владельца и интегратора, обновляет Rust/TS/JSON Schema/fixtures
и явно сообщается потребителям. Наличие DTO и компиляция не означают готовность реализации.
Результаты проверок исходного DTO-only `4cf8971`: [отчёт](fusion-creator-contracts-verification.md).
Это не свидетельство успешного QA текущего объединения; его проверки и повторные supervisor
тесты должны выполняться отдельно до публикации общего baseline.
