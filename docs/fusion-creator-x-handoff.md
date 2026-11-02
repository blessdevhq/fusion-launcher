# Поток X — compiler и declarative executor: handoff

База: `creator-wave2-base` (`5b3d611`). Ветка: `creator/executor`.
Первый опубликованный checkpoint: `ba4e1f2c40c42d4917994044bc9ec5f307fc3a2e`
(`WIP`, намеренно до проверки остатков timeout-подагентов).

## Статус и границы

Последний полный прогон после security fixes: `x-final-security-qa.log`,
`npm run qa`, **exit 0**: Node **159 passed**, Rust lib **656 passed / 4 ignored**,
packer **3 passed**, typecheck/static-check/build/fmt/clippy — PASS.
Результаты прежних прогонов ниже сохранены как история, не как текущий verdict.

Общий host-путь для локально нормализованных компонентов работает через существующие
CAS, workspace snapshots, точный byte approval и ProcessSupervisor. Это **не завершение
общей приёмки Creator и не доказательство реального цикла SuperTuxKart**.

В базе отсутствует descriptor-aware режим `external` для packer recipe и installation.
Существующий streaming HTTPS transport сам по себе не связывает такой recipe с frozen
origin, transport hash, `source_root` и normalized inventory. Эта работа принадлежит A.
X её не дублирует: исполнение descriptor с external-компонентом сейчас возвращает
`ActionUnavailable`, без старого launch fallback. Удалять этот gate следует только при
подключении и проверке handoff A. Сам compiler и локальная нормализация pinned ZIP
поддерживают проверку frozen origin, не выдают это за acquisition/installation.

Jobs, recovery, authoring/release pipeline и reuse stamps остаются у C; наследование,
относительные URL и recipe modes — у A. Новых installers, Source/Creator entrypoints,
параллельного transport, UI и platform-specific программы в switch не добавлено.
`eden_param_package_v1` не реализован и явно недоступен; remapping/input release ACK
не объявляются успешными. Forza остаётся бумажным случаем.

## Выполнено

### Compiler и одна проекция

- `fusion_creator/declarative/compiler.rs`: strict повторная проверка frozen DTO,
  ссылок, конечных build/control pairs, полного verified inventory, Windows aliases,
  main executable/cwd, typed scalar domains и mutable/immutable path conflicts.
- Выбор файлов делегирован существующему `projection::resolve_projection` B с
  `ProjectionSemantics::Strict11`. Отдельного merge/resolver в X нет.
- `candidate.rs`: только проверенные CAS inventories, pin nonmanaged inventory,
  полный перечень executable/loadable critical records, private candidate preparation.
- `plan.rs` и `workspace.rs`: explicit pair обязателен; декларативная ветка не смешивает
  legacy addon/runtime overrides. Legacy 1.0 ветка сохранена.

### XML и конфиги

- `codecs.rs`: `xml_attributes` использует frozen `xml_root` + direct-child
  `xml_attribute { elements, name }`, а не XPath или шаблонизатор команд.
- Проверены frozen SuperTuxKart XML seed → profile change, создание отсутствующей
  ветки, экранирование scalar, сохранение private/unmanaged значений.
- Fail-closed на неоднозначных addressed nodes/attributes, неправильном root,
  malformed XML, namespace/DTD/external entity constructs и превышении лимитов.
- После нового приоритета INI writer семантически не менялся: diff к WIP в его
  существующих функциях — только форматирование. Дополнительные guards общих
  typed settings и их тесты сохранены.

### Реальный supervisor и capabilities

- `runtime.rs`: literal argv остаётся literal, workspace path — один argv atom;
  command строится напрямую, без shell и без legacy template interpolation.
  Explicit cwd, cleared inherited environment, разрешённые host essentials и
  только одобренные workspace environment paths.
- `supervisor.rs`: общий descriptor использует тот же ProcessSupervisor, Windows Job,
  cancellation/descendant cleanup, output capture, exact-byte trust и save reconciliation.
  Перед native spawn проверяется PE signature + descriptor architecture; executable
  integrity locks уже удерживаются. Cwd/env directories также guarded.
- Critical integrity и UI approval inventory теперь используют **фактического
  winning provider** из snapshot projection, а не предположение «последний layer».
  Регрессия воспроизведена тестом до исправления.
- `capabilities.rs`: закрытая host-таблица `fusion.declarative@1.0` и
  `application_defaults@1.0`. Неизвестный id/version и отсутствующий optional writer
  дают `ActionUnavailable`. Нет Source-provided implementation или произвольной
  регистрации. Возвращается implementation BLAKE3, зависящий от кода host, shared
  execution/safety primitives и Cargo.lock, а не только ABI string.
  C может включить `ProgramCapabilityRegistry::require(...)` /
  `require_controls(...)` в свой reuse stamp; запись stamp/job в X не дублируется.

### Persistence и rollback

- `state.rs`: приватные физические copies; отсутствующий file не превращается
  в directory. Проверяются kind/path overlaps, reparse/junction, полный generation
  inventory и read-back hashes; fs2 lock сериализует store writers.
- Полный capture generation коммитится одним atomic latest pointer; generation
  immutable. Tombstones сохраняют удалённые/missing saves. Ошибка capture оставляет
  прежний latest, повреждение inventory — fail-closed до изменения candidate.
- Assemble/update/rollback и existing supervisor reconciliation используют этот store
  при существующем workspace lease. Старые binary snapshots восстанавливают latest
  state, а не старый save. Ветви legacy persistence остаются прежними.

## ZIP: негативные тесты и отсутствие второго normalizer

`io.rs` — host-policy seam вокруг уже существующих primitives, не новая реализация
распаковки/identity/resolver:

1. Whole-archive preflight проверяет names, Unix file types и resource limits,
   в том числе за пределами выбранного `source_root`.
2. Payload пишет существующий `fusion_source::transforms::ArchiveTransformer::extract`
   с `ArchiveFormat::Zip`, переданными `ExtractionLimits`, bounded streaming,
   containment checks и проверкой реальных размеров.
3. Exact selected root импортирует существующий `CasStore::import_directory`;
   normalized digest вычисляет CAS, тем же canonical inventory algorithm.
4. Mount/winner projection выполняет существующий resolver B, не ZIP wrapper.

Проверены: zip-slip; absolute `/`, drive, UNC и backslash names; точные дубликаты;
case conflicts; implicit-parent и file/directory conflicts; symlinks и прочие Unix
special files; entry count, compressed/uncompressed aggregate limits; per-member
ratio, настоящий Deflate high ratio, zero compressed-size anomaly; extreme ZIP64
lengths/checked arithmetic; поддельные declared lengths; staging cleanup; точный
source root; независимые transport и normalized inventory pins. Ограничения
host-only, caller не может обнулить или поднять их до произвольных значений.

Эта политика намеренно строже общих extraction primitives, но не копирует их
extraction loop, CAS identity или resolver. Recipe/download acquisition из A не
реализована здесь второй раз.

## Новое внутреннее snapshot-поле: env approval

`WorkspaceSnapshot.programEnvironmentApproval` — локальное optional поле с
`#[serde(default, skip_serializing_if = "Option::is_none")]`.

- Содержит canonical descriptor BLAKE3 и точный набор локально одобренных env names.
- Не часть portable ProgramDescriptor / SourceGame / CreatorProgramProject;
  source не может им выдать себе разрешение. Значений host environment или
  приватных binding values в portable carrier не добавлено.
- Задаётся только локальным host-кодом через `EnvironmentApproval::for_descriptor`
  и `WorkspaceAssembler::with_program_environment_approval`.
- Чтение старого snapshot без поля даёт `None`, не неявное approval.
- Descriptor с environment paths без соответствующего approval не запускается.

Доказательство совместимости:
`native_tests::legacy_h_snapshot_without_environment_approval_field_still_deserializes_and_verifies`
получает реальный legacy snapshot через H fixture, читает его без этого поля и
проходит прежний snapshot integrity guard. Native descriptor tracer также
доказывает сохранение approval через snapshot read/reacquire и отказ без exact
executable approval.

## Проверка timeout-подагентов и QA

По каждому набору просмотрен diff от базы и прочитаны новые файлы. Локальные diff
артефакты: `.tmp/creator-agent/review-{compiler,codecs,normalization,persistence}.diff`.
Все изменения после timeout проверены настоящим cargo test; timeout не считается
успешным handoff.

История результатов сохранена, а не переписана в PASS:

- `x-checkpoint-modules.log`: **FAIL**, `E0425`, отсутствовал `render_xml`.
- `x-module-audit.log`: **FAIL**, 38 passed / 2 failed; compiler negative tests
  обнаружили разрешённый shell host и workspace argument/path conflict.
- `x-capability-red.log`, `x-supervisor-red-corrected.log`,
  `x-provider-external-red.log`: RED для новых tracer/regression тестов.
- `x-provider-green.log`: **PASS**, 59 declarative tests, 0 failed.
- `x-full-qa.log`: **FAIL**, tests passed, затем два новых clippy замечания в тестах.
  Исправлены `cloned_ref_to_slice_refs` и cleanup readonly permissions без
  ослабления production security и без lint suppressions.
- `x-full-qa-final.log`: **PASS**, полный `npm run qa`, exit 0:
  - typecheck;
  - Node: **159 passed, 0 failed**;
  - static-check и frontend production build;
  - Rust lib: **645 passed, 0 failed, 4 ignored**;
  - packer bin: **3 passed, 0 failed**;
  - rustfmt check;
  - clippy all targets с `-D warnings`.

Логи находятся в `.tmp/creator-agent/` этого worktree. На Windows воспроизводимый
публичный command — `npm run qa` из Visual Studio Native Tools environment;
локальный runner только включает тот же toolchain, сериализует общий Cargo target
между worktrees и сохраняет stdout/stderr. Тесты не обходились и не отключались.

Реальный native tracer
`native_tests::native_descriptor_uses_existing_supervisor_exact_approval_and_latest_rollback_state`
использует существующее H environment и настоящий test PE: deny без byte approval,
запуск через existing supervisor, запись save, update и rollback с latest save.
Это не fake input/controller ACK и не заявление о запуске настоящего SuperTuxKart.

## Независимое ревью и исправления после первоначального QA

Первый independent review вернул **FAIL** по пяти пунктам, несмотря на зелёный QA.
Три назначенных fix-подагента остановились с HTTP 429; это не успешный handoff.
Их отчёты не использованы как доказательство исправления. Реальный worktree diff
проверен, недостающие исправления сделаны и повторно испытаны в родительской сессии.

1. **Undeclared loadables:** launch integrity перечисляет фактическое дерево,
   включая private/persistence roots, и отвергает executable/DLL вне verified
   critical inventory. Проверены отказ guard и отказ existing supervisor после
   добавления `app/version.dll` и DLL в saves. Winning-provider фильтр сохранён.
2. **Argument-only filesystem paths:** lexical validation недостаточна. Проверяются
   реальные ancestors/leaf, удерживаются directory/file guards. Missing target
   допускается на стадии metadata compilation, но fail-closed перед запуском.
   File argv является read-only input на время process lease; directory argv остаётся
   writable. Для writable/атомарно заменяемого config/save следует передавать его
   declared private directory, а не read-locked file argument. Это host-ограничение,
   не новый wire field. Junction-only аргумент проверен отдельным Windows тестом.
3. **ZIP indexing:** `zip_policy.rs` ограничивает EOCD/ZIP64, raw entry count,
   checked offsets и central metadata **до** реального library index. Metadata
   budget — `min(max_entries * 4096, 32 MiB)`; EOCD tail также bounded. Payload
   по-прежнему не буферизуется, extraction/identity/resolver не продублированы.
4. **Read-only active inspection:** active()/inspect_active() больше не вызывают
   program preparation/capture. Успешное и ошибочное чтение проверены сравнением
   hashes и file identities всего H environment: config/save/metadata не меняются.
   Preparation остаётся только для нового candidate/неактивного rollback target.
5. **Persistence history:** без explicit migration нельзя удалить или изменить уже
   записанную declaration. Отказ действует и перед capture commit, и перед
   rehydrate/activation — не только после запуска. Предыдущие latest bytes и
   tombstones сохраняются. Добавлять непересекающиеся roots можно; откат на
   descriptor, удаляющий записанный root, до реализации migration явно недоступен.

Повторное independent review закрыло пункты 1/2/4/5, но вновь вернуло **FAIL** для
ZIP: pinned library допускает EOCD внутри comment, заканчивающийся до EOF, и могла
выбрать не проверенную нами запись. Реальный regression
`zip_end_records_embedded_in_comments_cannot_bypass_index_budgets` показал принятие
такого архива при `max_entries=1` до исправления.

Последующее исправление отвергает альтернативные end-record signatures в metadata
и сначала проверяет выбранную интерпретацию тем же pinned parser на bounded
metadata-only view с исходными virtual offsets. View не раскрывает payload для
fallback search; разрешены только необходимые фиксированные 30-byte local headers,
не содержащие альтернативных end-record signatures. Лишь после успешного bounded
probe parser/extractor получает реальный, всё ещё deny-write locked archive.
Наличие подобных байтов в обычном payload само по себе не сканируется/не запрещается.
Multi-volume, ambiguous metadata и prepended-junk/SFX variants fail-closed.

Implementation fingerprints дополнены source bytes shared resolver,
program-schema validation и новым `zip_policy.rs`.

Дополнительная история реального исполнения (в `.tmp/creator-agent/`):

- `x-review-security-confirm-red.log`: 2 passed / **3 failed** до security fix.
- `x-review-security-green.log`: **65 passed** после guard fixes.
- `x-review-history-red.log`: history-loss regression **FAIL**;
  `x-review-history-green.log`: **10 passed**.
- `x-review-history-activation-red.log`: **1 failed** / 10 passed; затем activation
  history gate исправлен и входит в следующие успешные aggregate runs.
- `x-zip-index-red.log`: **2 failed** / 17 passed; `x-zip-index-green.log`: **19 passed**.
- `x-review-all-green.log`: **69 passed**.
- `x-review-full-qa.log`: **PASS**, Node 159 / Rust lib 655 / packer 3, 4 ignored.
- `x-zip-ambiguity-red.log`: comment-EOCD regression **FAIL** (архив был принят).
- `x-zip-ambiguity-green.log`: transient **FAIL**, 14 passed / 7 failed из-за
  отсутствовавшего fixed-local-header чтения в bounded probe. Ошибка не замаскирована
  изменением тестов; trace сохранён в `x-zip-probe-trace.log`, debug prints удалены.
- `x-zip-probe-complete.log`: **21 IO tests passed**.
- `x-final-security-qa.log`: **PASS**, полный QA: Node **159**, Rust lib **656**,
  packer **3**, **4 ignored**, fmt/clippy с `-D warnings`.

Финальный independent ZIP re-review `deleg_78d42480` завершён: **PASS**,
`security_concerns: []`, `logic_errors: []`. Проверены закрытие оставшегося
EOCD allocation finding, ограниченный metadata probe, удержание deny-write handle
и сохранение существующего streaming extractor/CAS пути. Это read-only code review;
ревьюер не запускал тесты и не читал дополнительные dependency sources.
Вместе с предыдущим re-review, закрывшим остальные четыре пункта, исходные пять
блокирующих замечаний закрыты. Полная приёмка продукта всё ещё ограничена
external/C/G/hardware пунктами, перечисленными ниже.

Неблокирующая рекомендация ревьюера: добавить комбинированный regression с
семантически повреждённым central directory и более ранним payload EOCD с excessive
entry count, отдельно проверяющий fallback-denial branch. Этот дополнительный тест
не добавлен; рекомендация не записана как выполненная.

Последнее независимое от полного QA подтверждение модулей:
`x-final-module-confirm.log` — **70 passed, 0 failed**, в том числе **21 IO test**.
Код staged tree сверён с проверенным деревом
`277f4baa4efcd0fb34da708d3d8b6a21723af0dc`; после него изменён только этот отчёт.

## Для следующего интеграционного шага

Нужен handoff A: recipe/install `external`, exact HTTPS/hash streaming acquisition,
связь selected root и normalized inventory, SHA и результат QA. Затем подключить
его в общий путь вместо временного `ActionUnavailable` gate и выполнить настоящий
SuperTuxKart install → run → profile change → update → rollback. Пока этот цикл
**BLOCKED / NOT RUN**, а не PASS. Реальная controller/InputDeviceSession приёмка,
Eden optional writer и интеграция C/G остаются отдельными неподтверждёнными частями.
