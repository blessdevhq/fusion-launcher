# Поток A — external acquisition (wave 3)

База: `creator-wave3-base`, `e30ee4b3950b3279dcbca7492579104968c75615`.
Ветка: `creator/external-acquisition`.
Worktree: `F:/MVP/Fusion-Creator-A-External`.

## Реализовано

- Planner больше не отклоняет весь descriptor из-за `origin: external`.
  `fusion_source/external.rs::lower_plan_layers` выводит один HTTPS ZIP source и
  BLAKE3 transport pin из frozen origin. Несогласованные уже заданные legacy
  transport fields отклоняются; исходный подписанный manifest не переписывается.
- `FusionService::run_install` перед обычным layer dispatch вызывает external
  acquisition, передаёт общий progress/cancellation и получает `LayerInput::Cas`.
  User-file/raw-path overrides внешнего компонента запрещены. Layer-only reuse
  для external намеренно не используется: origin/pins/source_root могут измениться
  при неизменном legacy layer. Успех acquisition сам по себе не даёт executable trust.
- Вызывается существующий `SecureHttpsTransport::fetch_with_progress` через
  host transport: bounded queue → файл → sync/close. Final HTTPS URL и фактический
  размер проверяются; существующий CAS digest verifier потоково проверяет transport
  bytes в том же blocking worker, который уже владеет всем private stage.
  Нового HTTP-загрузчика нет. `MirrorCascadeResolver` здесь не используется:
  frozen origin задаёт ровно один URL, а отдельный detached delivery verifier
  создавал лишнюю границу владения staging при drop async caller.
- Используется без изменения `declarative/io.rs::normalize_zip_with_limits`:
  whole-ZIP policy из `zip_policy.rs`, существующий `ArchiveTransformer`, точный
  `source_root`, существующая CAS inventory identity. Второго normalizer нет.
- Поскольку существующий X normalizer сравнивает inventory pin после импорта,
  его CAS здесь **приватный quarantine**. Только успешно проверенное дерево
  импортируется в CAS библиотеки. Quarantine/ZIP удаляются вместе с private stage.
  Цена изоляции — дополнительная дисковая копия, не буферизация архива в RAM.
- Host transport ceiling — 64 GiB, также ограничен `max_compressed_bytes` X;
  production extraction ceilings берутся из X defaults. Host caller может
  ужесточить лимиты для тестов. Descriptor не управляет этими лимитами.

## Точки подключения / владение

Новый production модуль — `src-tauri/src/fusion_source/external.rs`.
Минимальные существующие точки подключения:

- `fusion_source/mod.rs`: additive module declaration;
- `fusion_creator/declarative/runtime.rs`: удалён временный external gate,
  добавлен вызов lowering; код запуска процессов, codecs, persistence и supervisor
  не менялся;
- `fusion_source/service.rs`: additive external branch до legacy acquisition и
  исключение descriptor-owned external из `user_layers` (без изменения UI);
- `net.rs`: только `#[cfg(test)]` hook для TLS client configuration;
- `fusion_source/test_support/mod.rs`: additive test module declaration.

**Cargo.toml, Cargo.lock, package.json, package-lock.json, IPC, i18n и замороженные
Rust/TS/JSON Schema contracts не менялись.** Legacy Packer recipes и Source 1.0
не мигрируются. Managed/user_provided modes, inheritance 1.1, relative URL support,
UI, C jobs/reports и executor X не реализуются этим шагом.

Для интегратора C: этот шаг не подключает checkpoint/reuse. Перед использованием
acquisition как durable результата включить новый `fusion_source/external.rs`
в implementation stamp соответствующей операции; существующий список source
bytes в `ProgramCapabilityRegistry` этот модуль не перечисляет. ABI `1.0` сам
по себе не fingerprint acquisition. В текущей service-ветке external cache reuse
выключен: transport и inventory проверяются заново.

## Тестовый HTTPS-стенд

`fusion_source/test_support/https.rs` + `https/server.py` запускают loopback TLS
с авторским маленьким ZIP. Self-signed cert/key — **только тестовые fixtures**.
Доверие добавляется только в `cfg(test)` client для зарегистрированной пары
`acquisition.test:<случайный порт>`, без `danger_accept_invalid_certs`, без изменения
OS trust/env/DNS и без ослабления production redirect/URL policy. Стенд использует
Python 3 (`FUSION_TEST_PYTHON` либо `python`), новых Cargo/npm dependencies нет.

Позитивный тест начинает с пустого CAS библиотеки, получает signed source/manifest
и ZIP по настоящему HTTPS, не передаёт user files, проверяет normalized CAS,
selected plan layer и projected bytes существующей установки. Не выбранные
upstream siblings и префикс `source_root` не попадают в workspace.

Отрицательные update-тесты проверяют неизменность active snapshot JSON, hashes и
file identities активных файлов, latest save, отсутствие лишних library CAS objects,
пустой staging и успешную новую попытку после отказа:

- схема HTTP вместо HTTPS;
- credentials в URL;
- неверный transport BLAKE3;
- неверный normalized inventory BLAKE3;
- неполный Content-Length / TLS EOF;
- объявленный resource size выше ceiling;
- redirect HTTPS → HTTP;
- отсутствующий exact source_root.

Отдельно: chunked stream без Content-Length превышает byte limit;
shared-normalizer aggregate extraction/entry limits; неверный inventory в пустом
CAS не публикует объект и не создаёт snapshot.

Regression state: external placeholder не предлагает локальные inputs, но оба
override (`user_files` и `raw_paths`) всё ещё отклоняются; managed placeholder
сохраняет прежние требования. Regression drop: реальный HTTPS adapter завершает
writer, checksum worker синхронно удерживается в очереди единственного blocking
thread, acquisition abort-ится, затем очередь дренируется. Ни library CAS object,
ни snapshot не появляются; outer `acquire-*` staging полностью удаляется.

## Проверка / история

Команды запускаются из VS2022 Native Tools environment, с
`RUST_TEST_THREADS=4`, `CARGO_BUILD_JOBS=4`, offline Cargo и сериализованным shared
Cargo target. Node dependencies — junction на уже имеющийся доверенный
`node_modules`; чистый dependency install не заявляется. Исходники dirty launcher
и чужих worktrees не редактируются; общий Cargo target и orchestration lock
используются для сериализации проверок. Runner/logs: `.tmp/a-external/`.

- `a-plan-red2`: позитивный planner regression — **RED**, `ActionUnavailable`.
  `a-plan-red` был прерван во время компиляции, результата теста не было.
- `a-plan-green`: compile error в новом pattern (`instructions` не учтён);
  исправлено после чтения определения. `a-plan-green2`: **1 passed**.
- `a-https-red`: реальный clean-cache tracer — **RED**, прежняя ветка требует
  user-owned file. `a-https-green`: **1 passed** после подключения acquisition.
- `a-external-negatives`: **18 passed / 3 failed**. Ошибки ожиданий error codes и
  HTTP/1.0 chunked в тестовом стенде; production guards не ослаблялись.
- `a-external-negatives2`:
  `RUST_TEST_THREADS=4 cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib external_ -- --nocapture`
  — **21 passed / 0 failed**, exit 0.
- `a-full-qa-attempt1`: **FAIL**, после успешных Node 159 / Rust lib 670
  (4 ignored) / Packer 3 — rustfmt потребовал перенос новой test-only строки
  `net.rs`. Это formatting defect, не native flake; строка отформатирована.
- `a-clippy-check`: `cargo clippy --manifest-path src-tauri/Cargo.toml --locked
  --offline --all-targets -- -D warnings` — **PASS**, exit 0.
- `a-full-qa-attempt2`: `RUST_TEST_THREADS=4 npm run qa` — **PASS**, exit 0:
  Node **159 passed**, Rust lib **670 passed / 0 failed / 4 прежних ignored**,
  Packer CLI **3 passed**; typecheck/static-check/build/fmt/clippy прошли.
  SHA-256 raw log:
  `017bb2ec8e4e85c1585d7c76dface5031bf80617c6f94efa1c735917555bcc6d`.
  Этот PASS относится к дереву **до** последующих review fixes.
- Первое independent read-only review: **FAIL**, security concerns пусты,
  два logic findings: external placeholder ошибочно показывался в `user_layers`;
  drop acquisition future во время detached transport verifier мог оставить outer
  staging без cleanup owner.
- `review-state-red-01` → `review-state-green-01`: **RED → GREEN**, 1 test.
  Fix-подагент завершился по timeout до второго исправления; его незавершённый
  `review-abort-red-01` не считается результатом теста. Дальше работу продолжил
  основной агент.
- `a-review-abort-red-confirm`: **RED**, fully qualified test с `--exact`:
  после abort и завершения queued verifier оставался **1** outer staging вместо 0.
- `a-review-green-all`: `RUST_TEST_THREADS=4 cargo test --manifest-path
  src-tauri/Cargo.toml --locked --offline --lib external_ -- --nocapture`
  — **23 passed / 0 failed**, exit 0, после обоих fixes.
- `a-final-qa-after-review`: **PASS**, exit 0, полный
  `RUST_TEST_THREADS=4 npm run qa` после обоих fixes:
  Node **159 passed / 0 failed**, Rust lib **672 passed / 0 failed / 4 прежних ignored**,
  Packer CLI **3 passed**; typecheck, static-check, production build, rustfmt и
  clippy `--all-targets -- -D warnings` прошли. Native supervisor/sidecar прошли
  без падения; повтор из-за flake не требовался. Первый formatting FAIL выше
  сохранён и не переименован в PASS.
  SHA-256 raw log:
  `c92ddef8839be363619c6c17d1e6d50222cb8fe4e21412bfd209ecc6ba07b987`.
- `src-tauri` Git subtree до и после финального QA:
  `7a2bb2f7a8582a9dd64fd55d785afeced71619a5`. Проверено совпадение index/worktree;
  после QA разрешены только уточнения этого отчёта, не изменения исходников.
- Финальный independent re-review после fixes: **PASS**, `security_concerns: []`,
  `logic_errors: []`. Полный report: `.tmp/a-external/final-review.json`.
  Ревьюер прочитал полный staged diff и связанный код; builds/tests не запускал.
  Проверенный им `src-tauri` subtree совпадает с финальным QA выше. Два
  неблокирующих follow-up явно отделены от результата этого external patch ниже.

## NOT TESTED / не заявляется

- Реальные SuperTuxKart/Eden и полный install → launch → profiles → update →
  rollback цикл: **NOT RUN**, выполняет интегратор после слияния.
- Public upstream HTTPS/CDN/network/proxy interoperability, многогигабайтный
  archive throughput/disk exhaustion и hardware/controller acceptance: **NOT TESTED**.
- Отдельная синхронизированная отмена во время streaming body и уже активного
  hashing/normalization — **NOT TESTED** новым regression; он проверяет drop на
  границе queued verifier. Поздние ownership/cancel guards проверены чтением кода,
  мгновенное прерывание blocking IO и отмена уже начатой CAS publication не обещаются.
- Multi-component установка с выбранным profile, исключающим часть компонентов,
  не заявляется. Re-review обнаружил **прежний baseline integration gap**:
  `service.rs` получает только `plan.selected_layers`, а неизменённый
  `declarative/candidate.rs::verify_inputs` требует inventories всех components.
  Обе ветви проверены в `e30ee4b`; этот patch их не вводил и не исправляет.
  Интегратору B/X/C нужен отдельный subset-profile service scenario и согласование
  acquisition/checkpoint ownership, не ослабление общего compiler validation.
- Этот шаг не обещает general recipe authoring support, managed/user_provided
  acquisition, новые durable C checkpoints/reports или разрешение redistribution.
- Исправление известной нестабильности native supervisor/sidecar не заявляется;
  при падении в QA допускается один повтор, успешный повтор отмечается FLAKY.

## Изменённые контракты

Нет. Добавлен внутренний Rust acquisition API, wire versions/shapes не изменены.
