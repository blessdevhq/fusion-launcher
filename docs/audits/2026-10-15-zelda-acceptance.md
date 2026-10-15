# Fusion Launcher: проверка существующей Zelda TOTK на Windows

Дата: 2026-10-15. Окончательный код: `fa1f6264fc9fbd5519fb40b992d0a71ba9b7ef28`; основная интеграция: `9793eb708eb362fbe1c72a0967487590ecc98379`, ветка `wip/lifecycle-network`. Публичный релиз не выполнялся.

**Результат: два запуска через установленный GUI → настоящий Tauri backend → существующий Eden → настоящее анимированное титульное меню TOTK подтверждены. Оба процесса штатно завершились с exit code 0; повторный запуск не потребовал перезапуска Fusion.** Игровой мир и загрузка сохранения не проверялись. В Eden остаётся диагностический assert при OpenSaveDataFileSystem, описанный ниже; поэтому это PASS запуска до меню, а не полная игровая приёмка.

## Установленный GUI / Tauri: итоговая матрица

| Сценарий | Фактический результат | Evidence |
| --- | --- | --- |
| Port Game → выбрать прежний NSP → Use existing file | PASS: без копирования/полного хеширования | `gui-import-reference.png`, `library-after.json` |
| Сохранить локальную карточку, перезапустить/обновить acceptance Fusion | PASS: карточка и исходный путь сохранились | `gui-local-library.png`, `library-after.json` |
| Выбрать уже установленный Eden через Settings → Select | PASS: `switch-manual` указывает на прежний `eden-cli.exe`, `{game_path}` | `selected-emulator.json`, `gui-emulator-selected.png` |
| Checklist | PASS: Launch Ready, native prod.keys обнаружен без импорта; firmware optional | `gui-readiness.png` |
| GUI Play, первый запуск | PASS: PID **32672**, parent PID **18144** = установленный Fusion/Tauri; настоящее меню | `gui-run-1-start.json`, `gui-run-1-title.png` |
| Штатное закрытие первого окна | PASS: удержанный process handle подтвердил **exit 0** | `gui-run-1-result.json` |
| GUI Play повторно, тот же Fusion process | PASS: новый PID **15572**, тот же parent **18144**, настоящее меню/меняющиеся кадры | `gui-run-2-start.json`, `gui-run-2-title.png`, `gui-run-2-animated.png` |
| Штатное закрытие второго окна | PASS: **exit 0**, Eden-процессов не осталось | `gui-run-2-result.json`, `processes-after.txt` |
| Continue / New Game / игровой мир / запись игрового прогресса | NOT RUN | Continue и New Game не выбирались; загрузка игрового мира не проверялась |

Командные строки обоих процессов содержат единственный аргумент — исходный NSP. Родитель, executable и аргументы прочитаны через Win32_Process; наблюдатель процессов не запускает эмулятор и не завершает его. Запуски выполнены кнопкой Play в установленном Tauri GUI, а не preview/fake engine. Закрытие выполнено штатной кнопкой окна; `/F` и принудительное завершение для этих двух прогонов не применялись.

Локальная карточка: `user-library::the-legend-of-zelda-tears-of-the-kingdom`, `content_mode=user_provided`, `launch_json=null`, checksum NULL, torrent compatibility record `direct:user_link`, ownership config отсутствует. Реальная библиотечная ссылка не присваивает Fusion право удалить исходник. Удаление пользовательской игры в этой проверке не запускалось.

## Найденная установка

Поиск ограничен конфигурациями Fusion/Ryujinx, их каталогами Switch и подходящими именами каталогов в Downloads/HydraGames. Полное сканирование дисков не выполнялось.

- История использования **Eden v0.2.0, `eden-cli.exe`** подтверждена старым profile `switch-manual` основной установки Fusion и предыдущим Eden log с загрузкой TOTK/Vulkan. На нём выполнена текущая проверка.
- Также сохранился настоящий **Ryujinx 1.3.3+e2143d4**: `F:\RetroHydraTest\emulator\Ryujinx.exe`, 73 440 965 bytes. На старте именно он был выбран в acceptance-базе. Первоначальное предположение «stub» оказалось неверным и исправлено после чтения VersionInfo. Его EXE и данные не изменялись; запуск Ryujinx в этой проверке не выполнялся. Наличие второго эмулятора не позволяет утверждать, что раньше использовался исключительно Eden.
- Executable: `C:\Users\admin\AppData\Roaming\app.fusion.launcher\Emulators\switch\eden-cli.exe`.
- Native data: соседний `user/`; конфигурация `user/config/sdl2-config.ini`, ключи `user/keys/`, сохранения `user/nand/user/save/`.
- Игровой Title ID: `0100F2C0115B6000`.
- Существующая библиотечная копия: `C:\Users\admin\AppData\Roaming\app.fusion.launcher\Games\switch\manifest-underground-retro-archive--0100F2C0115B6000\The Legend of Zelda Tears of the Kingdom [NSP]\The Legend of Zelda Tears of the Kingdom [0100F2C0115B6000][v0].nsp`, **17 150 647 554 bytes**, заголовок `PFS0`.
- В том же каталоге есть файл `[0100F2C0115B6800][v131072].nsp`, **332 779 456 bytes**, также `PFS0`. Его наличие не доказывает применение обновления в текущем запуске; файл не устанавливался и не заменялся.
- Копия в Downloads: **15 485 468 672 bytes**, заголовок `00 00 00 00`, несколько начальных/средних выборок нулевые. Она непригодна для запуска как текущий NSP. Не удалялась, не исправлялась и повторно не скачивалась. Атрибут SparseFile сам по себе не является диагнозом: исправная библиотечная копия тоже sparse.

Содержимое ключей не выводилось, не копировалось и не включено в evidence. Сохранены только их имена, размеры и время изменения. Полный установленный firmware не подтверждён: в configured `user/nand/system/Contents/registered` файлов не найдено; Eden синтезирует некоторые системные архивы. Для наблюдавшегося титульного меню существующей конфигурации хватило; это не проверка всех игровых/applet сценариев.

## Настройки без их изменения

Из существующего SDL config: `nand_directory` и `sdmc_directory` ведут в соседний `user`, `use_docked_mode=1`, `resolution_setup=3`, `vulkan_device=0`, `use_vsync=2`, `gpu_accuracy=1`, disk shader cache и asynchronous GPU включены. Это исходные значения; их числовые enum не переинтерпретировались и не менялись.

Наблюдавшийся renderer: Vulkan 1.4.341, NVIDIA GeForce RTX 4050 Laptop GPU, driver 610.78.0.0. RAM машины около 16 GiB; во время первых запусков открыты два процесса Unreal Editor. Чужие приложения не завершались.

## Прямой запуск: фактические результаты

| Сценарий | Результат |
| --- | --- |
| Downloads NSP → Eden напрямую | FAIL: `Failed to load ROM (Error 52)`, `0007-0034`; в файле нет корректного начального PFS0 |
| Библиотечная копия, первый прямой запуск | FAIL: игра распознана, Vulkan начат, затем Windows APPCRASH `0x40000015` |
| Повтор той же библиотечной копии, без изменения настроек | PASS: настоящее титульное меню Tears of the Kingdom, пункт Continue, изменяющиеся кадры/FPS |
| Continue / New Game / игровой мир | Не проверялись; соответствующие пункты меню не выбирались |

Скриншот `direct-title.png` подтверждает саму игру, а не просто существование процесса. Direct повтор записывает memory samples и stdout/stderr. Первый crash не объявлен исправленным: одновременно средство захвата окна сообщило об отказе выделения памяти, но причинная связь с Eden не доказана. Последующий успешный запуск с теми же настройками исключает необходимость менять архитектуру Fusion ради этого сбоя, но не гарантирует стабильность эмулятора под любой нагрузкой.

Alt+F4 не дал немедленного подтверждения выхода. После нажатия штатной кнопки закрытия окна процесс исчез; последующий `taskkill /PID` без `/F` вернул «process not found». Force-kill для успешного direct запуска не применялся. Exit code direct успешного процесса не был сохранён после завершения наблюдения; для backend запусков требуется отдельное удержание process handle.

## Минимальные исправления Fusion

1. Локальный Port Game теперь может **использовать существующий файл на месте**. Новый optional `importInPlace` по умолчанию `false` для старых API-клиентов; интерфейс явно предлагает reference mode. Канонический путь записывается без копии и без ownership manifest. Удаление такой карточки не получает права удалить исходник.
2. Для этого пути inspection может пропустить полный hash большого файла. Интерфейс прямо сообщает, что checksum не вычислялась. Если ожидаемый SHA задан явно, backend всё равно проверяет его.
3. Existing native `prod.keys` выбранного Eden/Ryujinx обнаруживается только при отсутствующем Fusion-target. Ключ не перемещается и не копируется; существующий явно выбранный некорректный target не скрывается fallback.
4. 60 FPS выключен по умолчанию. Для этой проверки mods и shader URL не задаются; Fusion optimizer в vanilla Switch запуске не пишет конфигурацию, ключи, firmware или сохранения.
5. Ошибка импорта больше не игнорируется. Каталог и локальная ссылка записываются атомарно; optional checksum сразу сохраняется как NULL. SQLite trigger tests подтверждают rollback новой карточки и замены прежней при ошибке после первой записи download.

6. В установленном GUI обнаружено, что уже настроенный manual emulator нельзя выбрать повторно: видны только Open folder/Remove. Добавлена Select для ready manual profiles; регрессионный visual test настраивает профиль и проверяет, что повторный Select остаётся доступен. Профиль и файлы для смены EXE удалять не требуется.

Независимый Rust/Tauri review выявил два случая частичной записи и повторно проверил исправления. Новых блокеров в этом узком diff не найдено. Архитектура управления процессом эмулятора не перерабатывалась.

## QA кода

На чистых SHA `9793eb7` и окончательном `fa1f626`: **285 Rust passed, 0 failed, 3 ignored**, **98 Node passed**, clippy `-D warnings` и форматирование PASS. Три ignored теста относятся к прежней native libtorrent-приёмке, в этой задаче заново не запускались. `npm run mvp:visual` PASS; импортированная карточка просмотрена. В реальном установленном GUI просмотрены новый reference checkbox и выключенный 60 FPS; screenshots `gui-import-reference.png`, `gui-presets.png`.

На `fa1f626` visual regression дополнительно подтверждает, что manual emulator после перехода в Ready всё ещё имеет кнопку Select. Новых visual roles, canonical primitives или baseline approvals не создавалось.

## Артефакт и привязка к исходникам

Проверена отдельная установка **Fusion Launcher Acceptance**: `F:\MVP\Fusion Launcher\.tmp\acceptance\installed\fusion-launcher.exe`, product identifier `app.fusion.launcher.acceptance`. Обычная пользовательская установка `C:\Users\admin\Downloads\Fusion Launcher` не обновлялась; хеш её EXE совпадает с ранее сохранённым (`installation-preservation.json`).

Установленный launcher SHA256: `E59244799B9C6969124AAD57D0C77186BA1E05AA3CE6801D51B3C9D2A329D728`. Installer SHA256: `0274C561E7903F3A0F2C252B8D9C98B14140F8D3B99E70CDF0A991DB03812B98`.

Установленный EXE и оставшийся в `target/release` EXE имеют разные raw SHA. Потоковое сравнение выявило ровно три различающихся байта: маркер `__TAURI_BUNDLE_TYPE_VAR_NSS` против `__TAURI_BUNDLE_TYPE_VAR_UNK`. После нормализации исключительно этого NSIS-маркера файлы полностью совпадают; `bundle-marker-comparison.json`. В build log отдельно зафиксирован NSIS bundle patch. Это не выдаётся за raw hash equality. Хеши installer, installed/release launcher и sidecar сохранены в `sha256-artifacts.txt`.

Final QA и native evidence находятся в подкаталоге `fa1f626/`; `sha.txt`, пустые `status-before.txt`/`status-after.txt` фиксируют чистое дерево на проверенном коде. Документационный коммит после QA не меняет runtime. Финальный installer сохранён рядом с логами; это локальный acceptance-кандидат, не публичный релиз.

Сырые логи, снимки метаданных пользовательских файлов и скриншоты окон находятся в `.tmp/zelda-acceptance/2026-10-15/`. Они не содержат ROM payload, ключи, firmware или копии сохранений.

## Сохранность пользовательских данных

После прямых запусков:

- `prod.keys` и `title.keys`: имена/размеры/mtime совпадают.
- `sdl2-config.ini`: Eden обновил mtime, содержимое SHA256 совпадает с исходным.
- Содержимое игровых сохранений не изменилось. Eden изменил два небольших файла служебного cache metadata: `CacheStorageKey.dat` (9 bytes) и `CacheStorageInfo.dat` (24 bytes). Поэтому утверждение «во всём save tree не было ни одной записи» было бы неверным.
- `user/load` остался пустым; Fusion не добавлял FPS-моды.

Полный запуск игры, производительность игрового мира и длительная стабильность не выводятся из появления титульного меню.

После двух GUI-запусков повторная сверка дала те же границы: три NSP сохранили размер/mtime; ключи сохранили имена/размеры/mtime; SHA256 содержимого SDL config совпадает. Из 25 файлов в save tree изменилось содержимое только двух названных cache metadata файлов, остальные 23 совпали по SHA256. Новых/пропавших файлов нет. `user/load` остался пустым. Метаданные ROM проверены без полного чтения/хеширования всех 17 GiB; это не криптографический integrity audit payload.

## Открытые ограничения Eden

В обоих GUI-прогонах Eden записал `Debug <Critical> ... fsp_srv.cpp:289 ... assert false` при `OpenSaveDataFileSystem` (примерно 8–9 секунд после старта). Затем меню продолжало анимироваться, отображался FPS, процессы штатно вышли с кодом 0. Диагностика не скрыта: `gui-run-1-eden-log.txt`, `gui-run-2-eden-log.txt`.

Та же строка уже присутствует в успешном прямом запуске без Fusion (24.309 s, `direct-success-eden-log-excerpt.txt`). Поэтому этот симптом воспроизводится и вне Tauri; не следует приписывать его одному лишь запуску из Fusion или Windows canonical path prefix. Причина остаётся неизвестной.

Причина этого assert не установлена; работоспособность Continue/SaveData из меню не доказана. Настройки, firmware и ключи для его обхода не менялись, сохранения не восстанавливались/не заменялись, архитектура Fusion не менялась ради ошибки эмулятора. Отдельный первоначальный direct APPCRASH `0x40000015` также сохранён как незакрытый факт; связь с отмеченной нехваткой памяти остаётся гипотезой.

Условия общей готовности MVP (чистая Windows/VM, пользовательская сетевая матрица, длительный игровой сеанс) этой локальной проверкой не закрываются.
