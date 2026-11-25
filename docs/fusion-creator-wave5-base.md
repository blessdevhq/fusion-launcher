# Creator wave 5 — проверенный STK и инфраструктура C2

База: `creator-wave4-base` (`30c65aa`). Волну составляют `creator/stk-cycle`
(`dc804e81d07ab52b2b536de54b5d0bc222576d9f`) и C2
(`45e3239a2adc9bbd11b24a677f06795abd18d549`), merge без конфликтов `6a8354e`.
Интеграционное изменение `7e88929eb2df5ee6fa9fb09d7f34c6ad6cbfd393` форматирует
две сигнатуры макроса команд; меняющихся IPC имён или wire-полей нет.

## Реальный SuperTuxKart

[Отчёт и команды](fusion-creator-stk-cycle.md). Исход **В → PASS**: на базе
не хватало общей передачи локального approval из FusionService в workspace.
Повторный холодный install, XML, наблюдаемый процесс 10.4968064 с, update и
rollback прошли. ProgramDescriptor и замороженный контракт не менялись.
Источник проверки metadata — подписанный тестовый Source по loopback HTTPS;
ZIP пришёл с публичного pinned URL. Это не приёмка Creator GUI, Packer или G.

## Windows QA после слияния C2

- Отдельная ветка/worktree `creator/wave5-base`, `F:/MVP/Fusion-Creator-Wave5`.
- Полный `RUST_TEST_THREADS=4 npm run qa`: **PASS, exit 0**, на `7e88929`.
  Typecheck, static-check, Next build/export, rustfmt и clippy с `-D warnings`
  прошли. **190 Node-тестов, 706 Rust lib, 3 Packer CLI; 0 failures.**
  5 ignored: 4 прежних и opt-in STK, пройденный отдельной командой.
  В том числе пройдены восстановление после смерти процесса на пяти durable
  границах и отказ при изменившемся descriptor без нового approval.
- Ранний standalone C2 QA (`c2-full-qa-resumed.log`): **FAIL, exit 1**, 190 Node,
  688 Rust lib и 3 CLI прошли, затем rustfmt (две сигнатуры); Clippy не выполнялся.
  Интеграция исправила форматирование, а не переписала прежний FAIL. FLAKY не было.
- QA запускался с эксклюзивным `F:/MVP/.qa.lock`, VS2022 Native Tools и CMake в
  `PATH`, с общим Cargo target и junction на локальные Node dependencies.
  Raw log: `.tmp/wave5-qa-01.log`, SHA-256
  `4c1a2012a26cda600624ce48d30438aa235a2fa5433ee4274eb24a67dadef058`.
- После QA изменены только этот документ и требования инструментов в AGENTS.md.

## Границы волны

Проектные IPC и инфраструктура runner/recovery проверены, но production handlers
build/verify/export и связанные с ними IPC ещё `action_unavailable`.
F и E не слиты. Native GUI acceptance, mvp:visual, физический SDL/Eden,
production Packer/export и hosted HTTPS Source parity этой волной **NOT TESTED**.
