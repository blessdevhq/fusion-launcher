# Creator wave 1 — общая база, проверки и flaky policy

Статус: **объединённый исходный baseline проверен, контракт для волны 1 зафиксирован**.
Полный QA и серии 10+10 прошли; слоты начинают только после получения опубликованного тега
и проверки удалённого peeled commit по процедуре ниже. Creator v1 ещё не реализован.
Итоговая точка старта — commit, на который указывает **`creator-wave1-base^{commit}`**,
а не объект annotated tag и не старое имя `creator-base`.

## Что действительно объединено

| Источник | Как сохранён в истории |
| --- | --- |
| `creator/contracts` — `4cf897157cc9161a0432facd2dc9f32c6780f2d1` | Полный новый DTO/Zod/JSON Schema/fixture контракт, включая дополнительный `CreatorProfileSelection` |
| `creator/integrator` — `1f3116f` | Явный merge `8fa5898`: ранний standalone SDL tool и исходный native lifecycle H |
| `creator/input-native` — `0165f71` | Merge `721b9fe`: **реальный** SDL-модуль внутри приложения, virtual fixtures, probe, Windows workflow; ранее сохранённые reader1.1/profile-field gates |
| Общий H helper — `31008d027297274723820a15d10097726707f12e` | Отдельный scoped snapshot, merge `a624099`: один cfg(test) helper, reproducibility и root consumer |

Важное расхождение исходных предположений: один `creator/integrator` содержал только
standalone SDL и первую H-основу. Native SDL был в отдельной ветке; общий helper H оставался
незакоммиченным. Оба включены дополнительно, а не заменены описанием будущей интеграции.

Snapshot H создан временным Git index из строго семи проверенных H-файлов, с SHA-256,
совпавшими с ранее переданным evidence. Реальные HEAD/index/status/файлы исходного dirty
worktree H не менялись. Dirty integrator и исходный Fusion Launcher не сбрасывались/не переписывались.

## Решения по конфликтам: обе стороны сохранены по смыслу

- Семь add/add конфликтов native merge разрешены нейтральным reconciler:
  `fusionCreator.ts`, Rust `fusion_creator/mod.rs`, shared fixture, JSON Schema и schema tests
  оставлены побайтно равными новому контрактному superset `4cf8971`.
  Старые совпадающие объявления не дублируются, `CreatorProfileSelection` не потерян.
  В двух документах сохранён новый authoritative contract, а ограничения DTO-only коммита
  помечены историческими: текущий reader действительно принимает 1.0/1.1, optional profile IDs
  сохраняются в плане; выполнение 1.1/profiles пока честно unavailable.
- Три H-конфликта: расширенный shared-helper документ сохранён с историческим framing;
  исходные три install/request/native lifecycle test body сохранены в новом наборе;
  дублирующий in-test constructor вынесен в общий module, не заменён вторым installer.
  В `fusion_source/mod.rs` ровно по одной cfg(test) регистрации `test_support`
  и `creator_harness_tests`; остальные модули не потеряны.
- Native SDL worker и его fixture/test файлы не изменялись при разрешении конфликтов.
  Standalone tool сохранён как исторический отдельный crate, не используется как замена native SDL.
- Файлы исходных веток не удалялись; supervisor/sidecar/patch assertions, deadlines и ignored
  не ослаблялись ради зелёной базы. Нового resolver/job/export success stub нет.
- Compiler preflight после автоматического merge обнаружил два одинаковых объявления
  `ErrorCode::ActionUnavailable`: стороны добавили variant в разные позиции enum.
  Исправление `832648c503683986a2d4ec462b84ecd4f27446f9` убрало только повторное объявление,
  сам variant и wire code `action_unavailable` сохранены. Это дефект merge, не native flake.
  Первый preflight завершился до исполнения теста (INVALID, exit101), в статистику 10+10
  не включён; его raw log сохранён отдельно, новая полная серия начата после исправления.

## Два исторических native supervisor failures

Имена относятся к последнему contracts QA, не ко всем когда-либо падавшим Windows-тестам:

1. `fusion_source::supervisor::tests::native::partial_graph_startup_failure_stops_started_support_and_releases_lease`
   — `local executable lock remained after completion`, Windows sharing violation **error 32**,
   `supervisor_tests.rs:414`.
2. `fusion_source::supervisor::tests::native::cancelling_job_waits_for_actual_descendants_and_drops_all_base_locks`
   — **`descendant failed to publish pid`**, `supervisor_tests.rs:500`.

Ни один из этих тестов или его timeout/assertions в объединении не менялся.

### Серия из 10 отдельных запусков каждого

Режим: Windows/Rust1.95.0, `RUST_TEST_THREADS=4`, **последовательные отдельные Cargo invocations**,
без одновременно идущего QA/Cargo. Полное имя + `--exact`; каждый результат обязан содержать
ровно один исполненный тест, без ignored. Первый тест 10 раз подряд, затем второй 10 раз подряд;
сбои не обнуляют счётчик и не заменяются успешными retry. Build/0-tests считаются INVALID, не flaky.

| Тест | Исполнено | Успешно | Упало |
| --- | --- | --- | --- |
| `partial_graph_startup_failure_stops_started_support_and_releases_lease` | 10 | 10 | **0** |
| `cancelling_job_waits_for_actual_descendants_and_drops_all_base_locks` | 10 | 10 | **0** |

**Оба теста прошли 10/10; standalone-нестабильность в этой серии не воспроизвелась.**
Это не доказывает стабильность под общей нагрузкой и не устраняет прежние FAIL; их причина
остаётся UNCONFIRMED. Нельзя объявить дефект исправленным или исключить новую регрессию слота.
Committed JSON с каждым outcome/exit/числом тестов/временем/SHA-256 raw log:
[`evidence/creator-wave1-supervisor-repeats.json`](evidence/creator-wave1-supervisor-repeats.json).
Raw valid логи/JSON: `.tmp/wave1-supervisor-repeats-valid/`;
исключённый INVALID compiler preflight: `.tmp/wave1-supervisor-repeats/`.
Commit `832648c` фиксирует исходный код перед валидной серией; итоговый tag добавляет только документы/evidence.

Точная команда одного запуска (подставить любое из двух полных имён выше):

```sh
RUST_TEST_THREADS=4 cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib FULL_TEST_NAME -- --exact --nocapture
```

### Как отличать известный сбой от регрессии слота

1. Все выполняют `RUST_TEST_THREADS=4 npm run qa`, без новых skip/ignored или вырезания gates.
2. Исключение узкое: **упал только один** из двух перечисленных тестов, остальные tests/gates
   не падали, assertion совпадает с историческим fingerprint. Совпадение только имени недостаточно.
3. Сохранить первоначальный полный лог и base/own SHA; повторить этот точный тест **один раз**.
   Retry прошёл — записать **FLAKY (initial FAIL, retry PASS)**, не переписать исходный QA в PASS.
   Retry снова упал — **FAIL / блокер диагностики**, не бесконечно повторять до зелёного.
4. Если упали оба, другой тест/другой assertion, compiler/linker/setup, либо отсутствуют остальные
   gates из-за раннего выхода — не выдавать это за чистый QA. Явно перечислить FAIL/NOT RUN.
5. Если слот менял supervisor/launch/workspace/fixture путь, выполнить тот же repro в отдельном
   worktree **от неизменённого `creator-wave1-base`** с тем же окружением и параметрами. Не менять
   dirty рабочее дерево/stash ради baseline. Baseline тоже упал — свидетельство известной
   нестабильности, но не доказательство отсутствия своей регрессии; сравнить fingerprint/diff.
6. Не расширять allowlist на `graph_primary_crash...`, sidecar или patch cleanup автоматически
   лишь потому, что ранее встречались сходные Windows ошибки. Причина нагрузки/AV/teardown
   без диагностики остаётся **UNCONFIRMED**. Не увеличивать timeout и не отключать проверки как «фикс».

## Команды интеграционной проверки

Общее окружение: Windows, Rust1.95.0, доверенный Node dependencies junction;
`CARGO_TARGET_DIR='F:/MVP/Fusion Launcher/src-tauri/target'`, physical `FUSION_TEST_PYTHON`.
CMake **3.31.6-msvc6** из установленного VS2022 BuildTools CMake/bin добавлен в PATH.
Локальная ignored `src-tauri/target` существует для старых torrent fixtures,
даже если общий build cache вынесен наружу. Это не sandbox и не проверка чистого npm install.

| Проверка | Результат |
| --- | --- |
| `cargo test --manifest-path tools/creator-input-spike/Cargo.toml --locked --offline -- --nocapture` | **PASS: 1 fixture**, 0 failed/ignored; исторический standalone SDL |
| 20 exact supervisor invocations | **PASS: 10/10 + 10/10**, оба 0 failed/ignored; initial compiler INVALID исключён и сохранён |
| **`RUST_TEST_THREADS=4 npm run qa`** на объединённом коде | **PASS / exit0**: typecheck, **144 Node tests**, static-check, Next build/export, **550 Rust library tests + 3 CLI tests**, fmt, clippy `--all-targets -- -D warnings`; 4 прежних ignored |
| `cargo run --manifest-path src-tauri/Cargo.toml --locked --offline --bin fusion-launcher -- --input-spike-probe` | **PASS / exit0**, настоящий executable: SDL **2.26.4**, devices `[]`, `handles_closed: true`, initialized flags **0**; без auto-open physical device |
| `python .tmp/validate-creator-contracts.py` | **PASS**, conforming Draft202012Validator/meta-schema и wire fixtures, включая CreatorProfileSelection |
| Независимое read-only integration/plan review | **PASS**, затем отдельное review исправления enum **PASS**; не подменяет выполнение QA |

QA log: `.tmp/wave1-qa.log`; valid repeat runner: `.tmp/wave1-supervisor-repeat-valid-run.log`;
probe stdout/build stderr: `.tmp/wave1-native-probe.json`, `.tmp/wave1-native-probe-build.log`.
Старые `contracts-qa.log`, H/E reports/CI остаются историческими и не выдаются за текущий результат.
В полном QA прошли все шесть H/common-consumer scenarios, включая **реальный install → launch →
update → rollback с сохранением latest save**, и три `input::tests` с virtual controller/disconnect/
drop/ACK teardown. Два рассматриваемых supervisor-теста прошли и в этом общем четырёхпоточном QA.
Все source-файлы после проверки совпадают с `832648c`; финальный commit добавляет только
документы/evidence. Полный QA после dedup прошёл без retry; исторические FAIL не объявлены исправленными.

## Универсальная архитектура и три слота

[План](fusion-creator-v1-plan.md) обновлён: программу подключают проектом/декларативным описанием,
без per-program Rust/TS/IPC/platform registry правок и пересборки. Eden — первый проверочный профиль;
следом обязательна независимая реальная вторая portable-программа, не fork/fixture.
Input и Eden ParamPackage — прикладные capabilities, не requirement любой программы.
Безопасные конечные declarative operations не означают разрешение Source выполнять shell-скрипты.
Новые program descriptors/recipes не считаются реализованными только из-за этой записи в плане.

[Стартовые промпты](fusion-creator-wave1-prompts.md):
- **A+B** — один общий слот Source/Packer/installer/resolver, не два конкурирующих resolver;
- **C** — project/jobs/storage, номер migration выбирает сам как следующий свободный;
- **E** — продолжение имеющегося native input-спайка, не новый crate/worker или UI.

У каждого первая строка: «База: тег creator-wave1-base. Контракты зафиксированы в
docs/fusion-creator-contracts-v1.md (читай первым).» Один общий base SHA, собственные worktrees,
контрактные изменения через владельца/интегратора; Cargo/package/IPC registration — интегратор,
i18n — F. Нельзя начинать от старых branch names или наследовать чужие dirty изменения.

## Публикация и NOT TESTED

Тег `creator-wave1-base` ставится только после успешных gate-проверок на проверенный код + отчёт.
После push проверить `git ls-remote origin refs/heads/creator/contracts refs/tags/creator-wave1-base
'refs/tags/creator-wave1-base^{}'`: branch commit должен совпасть с peeled tag commit.
Не перемещать опубликованный base tag; новую базу объявлять новым именем и явно уведомлять слоты.

NOT TESTED этой подготовкой: реальный Creator v1 A–G pipeline, generic descriptor onboarding
и вторая реальная программа, physical controller/WebView2/Eden одновременное чтение/освобождение,
Eden/gameplay, native UI, external HTTPS cold acquisition, разные физические тома,
чистая dependency install и новый Windows CI run именно объединённой ревизии.
`npm run mvp:visual` не запускался: UI/CSS не менялись. Прежние ignored tests не считаются покрытием.
