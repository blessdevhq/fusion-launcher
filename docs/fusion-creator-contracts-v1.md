# Fusion Creator v1 — контрактный каркас волны 0

Авторитетный контракт объединённого исходного baseline Creator; Creator v1 ещё не реализован.
Исходный DTO-only коммит `4cf8971` ветки `creator/contracts` от `creator-base`
(`287dc370b891577c5c5ca27ac9126766daf89920`) имел только контрактный scope A–H.
Утверждения о его узких границах ниже относятся к истории этого коммита, а не к текущему
объединённому baseline, включающему `creator/integrator`, native SDL и общий H helper.
**Контракты зафиксированы для `creator-wave1-base`; полный объединённый QA с четырьмя test threads
прошёл.** Точный состав/результаты/известная нестабильность — [отчёт базы](fusion-creator-wave1-base.md).
Это готовность базы к работе слотов, не готовый Creator v1 или универсальное подключение программ.
Исторические результаты и NOT TESTED scope сохранены отдельно в
[fusion-creator-wave0-audit.md](fusion-creator-wave0-audit.md) и
[fusion-creator-wave0-handoff.md](fusion-creator-wave0-handoff.md); они не заменяют QA объединения.
Нормативная разбивка потоков: [план v1](fusion-creator-v1-plan.md).

**Дополнение `creator-wave1-descriptor`: формат ProgramDescriptor 1.0 заморожен ниже и в JSON Schema.**
Это **JSON/document contract freeze**, не реализация новых Rust/TS DTO, reader или executor.
`creator-wave1-base` не перемещается. Сверка двух программ, первоисточники, реальные pins,
расхождения с кодом и границы проверки — [descriptor evidence](fusion-creator-descriptor-evidence.md).

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
- `AdapterReference { id, version }` не ограничен `eden_portable`: новая обычная portable-программа
  подключается данными/декларативным описанием, без нового Rust/TS module, IPC/program enum,
  platform registry или пересборки Fusion под её имя. Eden 0.2.1 — первая проверка общей системы,
  затем требуется независимая реальная вторая программа; synthetic H не заменяет эту приёмку.
  Общий descriptor и безопасные declarative layout/config recipes реализуются по обновлённому плану,
  не выдаются за уже работающую возможность этого baseline. Новые поля DTO — только через владельца
  и интегратора; input/Eden ParamPackage не являются обязательными capabilities любой программы.
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

## ProgramDescriptor 1.0 — нормативная заморозка

### Документы, версии и единственный источник истины

- Самостоятельный snake_case документ: `$defs/ProgramDescriptor`,
  `document_type: "program_descriptor"`, `schema_version: "1.0"`.
  Не путать с GameManifest 1.1, SourceIndex 1.0 и версией самой программы.
- `identity { id, title, version, os, architecture }`: точная версия программы, Windows и
  одна из `x86_64|x86|aarch64|armv7`. Версии непрозрачные точные строки, не `latest`/диапазоны.
  `executor { id: "fusion.declarative", version: "1.0" }` — версия общего host API,
  **не** Eden `0.2.1`, SuperTuxKart `1.5` или версия игры.
- Versioned authoring carrier — `$defs/CreatorProgramProject`, `schema_version: "1.1"`:
  `project_id, revision, title, source_id, game_id, game_version, program_descriptor`.
  Последние game-поля — metadata библиотечного выпуска, не compatibility requirements.
  Descriptor единолично владеет components/profiles/rules: дубликатов `adapter`, `components`,
  `build_profiles`, `controls_profiles`, `projection_rules` на уровне этого проекта нет.
- Старые `$defs/CreatorProject` 1.0 и `$defs/CreatorManifest` **не изменены**. Новые поля нельзя
  подмешивать в строгий legacy DTO или прятать в строку `adapter.id`. Миграция только явная.
- Для публикации `$defs/ProgramCreatorManifest` — новая строгая альтернатива `GameManifest.creator`:
  **ровно** `{ program_descriptor: ProgramDescriptor }`. Legacy CreatorManifest и эта альтернатива
  взаимно исключаются. Внешний GameManifest остаётся 1.1, SourceIndex остаётся 1.0.
  Descriptor встроен inline: canonical manifest hash и существующая Ed25519 signature покрывают
  его содержимое. Никаких незакреплённых sidecar URL/дескрипторов, загружаемых после проверки подписи.
- Это нормативная целевая форма: текущие Rust/TS project DTO и metadata reader новую альтернативу
  ещё **не принимают**. A/C/D обязаны согласованно добавить типы, canonical/storage и потребителей;
  нельзя включать execution до этого. Производные legacy runtime/launch/persistence поля, если
  нужны transport/planner, генерирует общий compiler; расхождение с descriptor — ошибка, не выбор
  «более удобной» копии. Старые 1.0 публикации не мигрируют неявно.
- Release/report/resume привязаны к immutable release и выбранной паре профилей. C добавляет
  canonical descriptor BLAKE3 в существующий `stamp.inputHashes.program_descriptor` **до reuse**,
  наряду с project revision, component hashes и executor/capability implementation digests
  (ABI version сам по себе не fingerprint кода). Приватный binding revision/hash также входит
  в локальный stamp и не экспортируется. Изменение любого
  из них инвалидирует зависимые результаты. Пять durable границ ниже не меняются.

### Что описывается данными

| Поле | Замороженная семантика |
| --- | --- |
| `components[]` | `id, layer_id, origin, source_root, mount_path`. Origins ровно прежние managed/external/user_provided; применяются независимо к каждому компоненту. |
| `source_root` | Точный slash-путь **внутри выбранного input/archive root**; пустая строка означает этот root. Проверить весь архив перед доступом; выбрать этот каталог, нормализовать его файлы. Не auto-strip, поиск exe, glob или hook. |
| `mount_path` | Путь в inactive workspace для полного проверенного нормализованного `CasFile` inventory; пустая строка — workspace root. Сам mount не меняет inventory identity. |
| `required_files[]` | Обязательные projected paths и роли data/main_executable/core_library. Не полный inventory и не allowlist единственных проверяемых DLL: все исполняемые/loadable файлы проходят общую critical-file policy. Хеш/размер/provider берутся из проверенного inventory и общего resolver, не из имени. |
| `projection_rules[]` | Точное `path → winning_layer`, одна семантика A/B/D/G. Different bytes без winner — конфликт; dependencies и порядок components не приоритет. |
| `directories[]` | Конечные relative directories, которые host создаёт/проверяет внутри candidate, включая portable marker и приватные data dirs. Не reparse/junction, не создание файлов вместо каталогов. |
| `entrypoint` | Один PE exe из verified projection, explicit working_directory, конечный argv и `environment_paths`. Запуск через общий supervisor, не через shell/wrapper. |
| `arguments[]` | Либо literal argv atom, либо `{ kind: workspace_path, path, prefix }`: host разворачивает путь внутри workspace в **один** argv atom. Prefix пустой или `-option=`/`--option=`. Это не шаблонизатор. Авторизованные user files сначала становятся проверенными компонентами. |
| `environment_paths[]` | Имя application data/config variable и workspace-relative directory. Host выставляет абсолютный путь только дочернему процессу. Никаких literal env values, изменения глобального environment или loader/search-path переменных. |
| `configs[]` | Файл, codec, seed, `preserve_unmanaged: true`, конечный allowlist простых ключей. Только UTF-8 INI section/key или XML root/child-elements/attribute, без XPath/XSLT/DTD/includes. |
| `build_profiles[]` | Конечные ID/title, точные layer IDs, допустимые controls profile IDs, полный конечный набор `settings {config_id,key_id,value}`. Каждое допустимое сочетание проверяется до сборки. |
| `controls_profiles[]` | Либо явные `application_defaults` без Fusion remapping, либо известный host writer `eden_param_package_v1` с логическими bindings/device/config_id. Native defaults не притворяются SDL capture или сериализованным назначением. |
| `persistence[]` | Точный file/directory path, `preserve_latest`, `export: never`. Отсутствующий **file** не превращать в каталог. Restore/update/rollback сохраняют актуальный progress, не заменяют его старым snapshot. |

`configs.keys`: ID, typed address, scalar_type `boolean|integer|number|string`, непустой конечный
`allowed_values`. Значения — literal scalars, без вычислений и интерполяции. Integer — safe JSON
integer; number — конечный bounded JSON number. Host проверяет согласованность scalar_type/domain,
принадлежность profile values этому domain и полное однократное назначение каждого объявленного
ключа во всех build profiles. Объявления ключей/адресов уникальны; codec/seed/address совместимы.
INI сохраняет точные section/key, включая literal `\default` у Eden. Boolean сериализуется
как `true`/`false`, числа — locale-independent decimal, строки — escaped codec literals.
XML address — последовательность
прямых child names относительно root и один attribute; отсутствующие простые nodes создаются,
неоднозначные duplicate nodes/attrs, DTD/entities/namespaces и неподдерживаемый формат — ошибка.
Seed используется только при отсутствии приватной конфигурации: empty_ini или минимальный xml_root
с конечными root attributes. Existing unrelated nodes/keys сохраняются, но **никогда не экспортируются**.

### Что остаётся кодом за общим host interface

Общий executor валидирует descriptor/references, нормализует input, вызывает единственный resolver,
готовит writable candidate, применяет простые codecs, формирует argv/env и использует существующие
supervisor, CAS, snapshots, trust и release pipeline. Имена программ не появляются в switch/enum
IPC, platform registry или импортируемом production module для каждого нового portable приложения.

Adapter API 1.0: host выбирает **уже зарегистрированную** capability по закрытому ID/version,
передаёт descriptor digest, выбранную пару профилей, config ID, логические назначения и приватное
read-only binding view; capability возвращает план config mutations либо typed unavailable/invalid
diagnostic. Source не передаёт implementation, function name, URL или native module. Host повторно
проверяет output paths/key ownership и атомарно применяет план **только в inactive candidate**.
Это сигнатурный контракт следующей реализации D, не существующий Rust symbol/stub.

Eden ParamPackage escaping, `[empty]`, вложенные stick packages, Qt/native keycodes, SDL mapping,
локальные device identities/calibration и размещение конкретных `[Controls]` записей остаются
host-owned кодом D/E. В v1 optional writer — `eden_param_package_v1`; его API version фиксирован
именем capability. Больше никаких произвольных adapter actions. Shared INI/XML writers — общий
код, а не адаптеры под каждую программу. Отсутствие capability не означает native/default fallback:
недоступная выбранная операция возвращает `action_unavailable`, job не становится successful.

### Проверки и безопасность (обязательны сверх JSON Schema)

1. До parse — лимит descriptor 2 MiB и duplicate JSON object keys rejection. Схема закрывает
   все object shapes и ограничивает списки; unknown versions/opcodes/properties — ошибка.
2. Проверить уникальные component/layer/profile/config/key/env IDs, все references и выбранные
   пары. Прогнать pure resolver для каждого build profile: required files, entrypoint и его
   executable role должны реально существовать с правильным winning provider; cwd — каталог.
3. Проверить все source/normalized/projected paths, parent/file и case collisions, Windows
   device/reserved aliases, traversal/absolute/UNC/ADS/control chars. Reparse/symlink запрещены;
   containment и TOCTOU проверяются при каждом IO. Schema pattern — лишь lexical prefilter.
4. Только конечные host operations: select exact input root, normalize/verify, project, ensure-dir,
   typed scalar assignment, approved capability config plan, persistence copy/commit и supervisor
   spawn approved PE. Никаких Source shell scripts, `.bat/.cmd/.ps1` entrypoints, cmd/PowerShell/
   script-host launchers, eval, WASM/native plugin или post-setup escape hatch. Наличие штатного
   `.bat` в upstream ZIP не разрешает запускать его Fusion; descriptor указывает настоящий exe.
5. Literal argv не исполняются shell. Не наследовать application-specific path overrides из
   родителя; protected APPDATA/LOCALAPPDATA/TEMP и loader environment задаёт host, Source не меняет.
   Env map ограничен workspace paths и generic protected/loader policy. Новые application data
   variable names объявляются данными и проходят локальное approval, не требуют program-name
   allowlist в Rust. Опасные/недекларированные overrides — отказ, regex не выдаёт permission.
   Не передавать signing secrets и private binding storage.
6. Внешняя загрузка streaming-to-disk, expected transport и normalized inventory BLAKE3 проверяются
   отдельно. User-provided inputs требуют явного локального выбора, ожидаемого inventory и
   проверки копии; public descriptor не содержит absolute author paths, ROM/BIOS/keys/firmware.
7. Сохраняются Ed25519 verification, TOFU/explicit key-change approval и executable approval
   actual winning exe/core-library bytes. Подпись не даёт license или executable approval;
   HTTPS/upstream checksum не заменяют Source trust. `redistribution_allowed` не разрешение.
8. Все mutable config/persistence paths получают private writable copies **до** редактирования
   и запуска: не менять shared CAS hardlink/inode permissions. Persistence сначала rehydrate в
   candidate, затем profile-managed keys; active config/shared saves до commit не трогать.
   После остановки capture/atomic commit в приватный durable state. На rollback — latest state
   плюс выбранный старый profile, не восстановление прогресса из старого release.
9. File persistence может совпасть с exact config file: общий writer сохраняет unmanaged private
   values, затем overrides managed keys. Остальные overlapping persistence roots, file/dir conflicts
   и collisions configs/directories с immutable projected files — отказ, не last-writer-wins.
   Отсутствующие config parent directories host создаёт безопасно; отсутствующее сохранение — no-op.
10. Export собирается из allowlisted release artifacts/metadata, **не из запущенного workspace**.
    Persistence, приватные config copies, tokens, GUID, caches/logs и пользовательские input paths
    не экспортируются. `application_defaults` не отменяет закрытие прежней input session с worker ACK.
11. Это защита операций Fusion и explicit approval, **не OS sandbox произвольного одобренного PE**.
    Документированные savedir overrides и source inspection не доказывают отсутствие любых внешних
    записей запущенного процесса. Для observed_launch/manual_scenario нужны реальные отчёты.

### Две программы и цикл без app-specific production changes

- Eden 0.2.1: точные portable directory/config/simple INI keys — данные; ParamPackage — optional
  capability. [Eden fixture](../fixtures/creator/programs/eden-0.2.1.json) — **schema-only**, с
  unresolved managed artifact и `redistribution_allowed: false`, не опубликованный runtime или
  полный готовый mapping. Ни game payload, ни keys/firmware не добавлены.
- [SuperTuxKart 1.5 fixture](../fixtures/creator/programs/supertuxkart-1.5.json) — не эмулятор:
  реальные ZIP roots/pins, x64 exe/cwd, `SUPERTUXKART_SAVEDIR`, XML attrs и native-default controls.
  В бинарном Source не нужны `supertuxkart` enum/adapter/IPC/registry branches.
- Целевой полный цикл: C сохраняет scan/classification и descriptor pin → A/D получают проверенный
  artifact/inventory → общий compiler и resolver собирают все объявленные profile pairs → B/D
  готовят candidate, XML profile и приватное состояние → E освобождает handles → supervisor
  запускает **exe** после trust/approval → G сохраняет отдельные structure/install/observed/manual
  reports → export содержит signed descriptor/manifest и allowed metadata, external payload остаётся
  pinned upstream dependency → update строит новый candidate → rollback выбирает прежний release
  с latest persistence. Новый выпуск требует новых exact byte pins, не изменяемого `latest` URL.
- **На текущем baseline этот цикл НЕ выполнен и без общей реализации executor не доступен.**
  «Без правки production-кода» означает отсутствие изменений **под имя новой программы** после
  реализации общего D/A/B/C/G pipeline, не утверждение, что схема уже включает исполнение.
  Не считать schema validation, скачивание ZIP, synthetic H или showcase реальной приёмкой.

### Forza Horizon 2 + Azimuth — только анализ выразимости

Подтверждённые upstream требования/права и список пробелов вынесены в
[evidence, бумажный кейс](fusion-creator-descriptor-evidence.md#forza-horizon-2--azimuth--только-на-бумаге).
Программа/runtime version отделена от game metadata и executor/capability version. Origins и
проверенные component inventories позволяют описать пользовательские файлы и уже готовые
компоненты; они **не** выражают Xbox Title ID/update/content resolver, требуемый binary patch
pipeline или неизвестный launch/config API. Эти пробелы **не расширяют объём v1**.
`forza-horizon-2-showcase` остаётся synthetic fixture, не real compatibility/permission evidence.

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
| Общие декларативные program/layout/config recipes, первый Eden-профиль и ParamPackage | **D**, общий формат согласуется с A+B. Подключение новой программы не требует правки production-кода под её имя. |
| `fusion_creator` project/jobs/reports, `commands/fusion_creator.rs`, migration | **C**. G подключает handlers к C runner/storage, не создаёт конкурирующую миграцию. |
| `fusion_creator/input.rs`, native input service | **E**, SDL worker ownership; input wire-изменения согласуются с C/F/D и интегратором. |
| `src/types/fusionCreator.ts`, JSON Schema и shared fixtures | Интегратор замораживает контракт; дальше **C + F**, с A/B/D/E по их DTO. Менять Rust/TS/schema/fixtures согласованно. |
| Общий integration/native harness | **H/H2**; unit-тесты реализаций остаются у A–G. SDL/H foundation включён и проверен общим QA; использовать один cfg(test) helper без копирования installer/transport. |

| Поток | Namespace, которым владеет автор текста (вносит F) |
| --- | --- |
| A | `creator.source.*` |
| B | `creator.install.*` |
| C | `creator.project.*`, `creator.job.*` |
| D | `creator.adapter.common.*`, `creator.adapter.eden.*` |
| E | `input.*` |
| F | `creator.ui.*`, `creator.common.*` |
| G | `creator.release.*`, `creator.verify.*` |
| H/H2 | нет production-ключей |

Изменение frozen DTO проходит владельца и интегратора, обновляет Rust/TS/JSON Schema/fixtures
и явно сообщается потребителям. Наличие DTO и компиляция не означают готовность реализации.
Результаты проверок исходного DTO-only `4cf8971`: [отчёт](fusion-creator-contracts-verification.md).
Это не свидетельство успешного QA текущего объединения. Его отдельный полный QA, серии 10+10,
native probe и условия опубликованного baseline — в [отчёте волны 1](fusion-creator-wave1-base.md).
