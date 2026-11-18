# SuperTuxKart 1.5 — проверка универсальности Creator

База: `creator-wave4-base` (`30c65aaeae770fd9f238b989485bb5f07332f1dd`). Отдельный worktree `F:/MVP/Fusion-Creator-STK`, ветка `creator/stk-cycle`. Замороженный `fixtures/creator/programs/supertuxkart-1.5.json` не изменён.

## Воспроизведение

Из VS2022 Native Tools с CMake в `PATH`, `RUST_TEST_THREADS=4`, `FUSION_TEST_PYTHON` (физический путь к Python), `FUSION_STK_TEST_ROOT` (существующая scratch-директория на локальном NTFS со свободным местом и доступная из графической Windows-сессии):

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib fusion_creator::declarative::provider_tests::stk_cycle::real_supertuxkart_clean_install_observed_launch_update_rollback -- --exact --ignored --nocapture
```

Обычный QA специально не запускает этот сетевой тест. Он создаёт временные SQLite, library/CAS и рабочие generations, подписывает индекс локальным тестовым ключом, подаёт индекс и манифест по loopback HTTPS через существующий fixture. **Сам ZIP** получает `FusionService` по неизменённому публичному URL поставщика; нет fixture payload, upfront seeding, HTTP fallback или bypass хешей. Список 92 loadable-файлов для внешнего манифеста получен из ранее вычисленного полного normalized inventory (описан в [descriptor evidence](fusion-creator-descriptor-evidence.md)); их байты не коммитятся. Это hand-authored тестовый Source, **не Packer/Creator export**.

## Исходный прогон: В — блокер установщика

`a45ecf86f3f3b9ea177488e14696180e98cdcbae`, `.tmp/stk-baseline.log`: тест стартовал, signed Source принят и TOFU identity записана. С пустым CAS передано **922 430 727 байт**; normalizer записал объект `9f1b886deb979f6b6c2d59b4d6ae820faf5f900b5877066e8798300e57676bf9`. На сборке workspace получен `ActionUnavailable: Application environment paths require approval for this exact descriptor.`; **FAIL**, cargo exit 101. XML, native launch, update и rollback в этом прогоне не достигнуты.

Причина: `WorkspaceAssembler` уже принимает локальное решение для точного digest descriptor и передаёт его в snapshot, но `FusionService` до исправления всегда создавал assembler с `None`. Тестовый оператор одобряет ровно три конкретных имени; production не получает список от Source/IPC, не обходит проверку, не содержит специальных имён этой программы. Новый узкий host API не является UI или persistent approval store.

## Повторный цикл: PASS после общего исправления (исход В → PASS)

Первый повтор с рабочим host approval (`.tmp/stk-host-approval.log`, exit 101) подтвердил
чистую установку и XML. Наблюдатель процесса упал до результата: PowerShell
`Get-Process -Name ... -ErrorAction SilentlyContinue` возвращает 1 без stderr,
когда процесс ещё не появился. Это воспроизведено отдельно; исправлен только probe.

Финальный `.tmp/stk-observer-fixed-live.log`, команда из раздела выше: **1 passed / exit 0**.

| Шаг | Результат |
| --- | --- |
| Source + TOFU | Подписанный release `1.5-creator.1`, raw manifest BLAKE3 `7449b07b2fc8d3ac3baf006415c2e7075aa250e0651bbdca8aabf1512e6a4c1d`. |
| Холодная установка | Пустой CAS, 922 430 727 потоково скачанных байт, обе проверки пинов, 5 763 файла в нормализованном CAS `9f1b886deb979f6b6c2d59b4d6ae820faf5f900b5877066e8798300e57676bf9`. |
| XML и наблюдаемый запуск | Ключи fullscreen/show_fps/music в XML для `windowed`/`native`. До явного exact-byte trust запуск отклонён. После одобрения PE именно из candidate наблюдался **10.4968064 с** (PID 34464), затем остановлен через `game_cancel`. |
| Обновление | Подписанный release `1.5-creator.2`, raw manifest BLAKE3 `72e1c7a8351b9e5698a0abb733ecfc5ba8c4e9bb61307d832c117de08720de63`; другой snapshot, `fullscreen`/`native` и соответствующий XML. Сохранён приватный контрольный файл. Upstream ZIP и версия программы не менялись. |
| Откат | Восстановлены первый snapshot и `windowed`/`native`, XML снова соответствует старому профилю. Контрольный файл сохранил байты, записанные после обновления, а не старую копию. |

`RUST_TEST_THREADS=4 cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib fusion_creator::declarative::provider_tests -- --nocapture`: **20 passed / 0 failed / 1 opt-in ignored**, включая отрицательную проверку смены descriptor без нового approval и регрессию probe. Архив и ключ выпуска вне Git. Это не ручной игровой сценарий, не проверка публично размещённого Source и не приёмка GUI.

## Список пробелов

- Expressiveness: для пройденного цикла не потребовалось менять `ProgramDescriptor` или добавлять код под имя приложения. Блокер базы относился к общему installer wiring, а не к выразимости описания.
- Authoring: тест создаёт Source shell вокруг descriptor и полного проверяемого списка loadables вручную. Генерация через Packer, приватные approvals в Creator/Launcher UI и экспорт этим тестом не проверяются.
- Placement: установщик по умолчанию размещает managed/external workspace в `library/.fusion-source/workspaces`; внешний путь из InstallRequest предназначен для явно настроенной legacy local-base placement. Нативный тест использует стандартное размещение в отдельной временной библиотеке, а не штатную библиотеку игрока.
