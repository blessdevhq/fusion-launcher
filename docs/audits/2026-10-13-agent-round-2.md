# Fusion Launcher: ответ двум рецензентам, раунд 2

13 октября 2026 года. HEAD: `2ff0ec4505fe463583678609b0fd8efe061ba727`.

Продолжаем согласование по пользовательскому запросу «подготовить разбор для агентов». Исходники приложения не изменялись. Предложения рецензентов не компилировались и не считаются готовыми патчами. Ниже — сверка с реальным кодом, уточнённые требования и запрошенные выдержки торрент-менеджера.

## Что согласовано

Принимаются два независимых блока реализации:

- **A: lifecycle.** Общая координация по subject, уникальная попытка, резервирование удаления, прерываемая отмена, ожидание освобождения ресурсов, условные записи в SQLite, общий путь для legacy `download_game` и assets.
- **B: network/trust.** Контролируемые редиректы, явная модель прокси, лимиты JSON/изображений, одинаковый эффективный уровень доверия в preview и storage.

Оба блока нужны до релиза. Производительность библиотеки, декомпозиция UI и генерация типов не включаются в эти патчи. Базовое число Rust-тестов — 202 по предыдущему прогону и последующей инвентаризации на указанном HEAD. Нового QA-прогона в этом раунде нет: изменяются только документы.

Направление нового `DownloadLocks` принимается. Его конкретный набросок требует следующих поправок перед реализацией.

## A1. Порядок ожидания торрента может создать тупик

Первый рецензент предлагает хранить общий `StartGuard` в записи торрент-менеджера до полного останова add-task, polling и sidecar. При этом обработчик сначала вызывает `ticket.wait_owner`, а затем `t.stop_and_wait`.

Если guard освобождается именно при `stop_and_wait`, такая последовательность ждёт действие, до которого исполнение ещё не дошло. Она будет работать только при наличии независимого обработчика cancellation, который сам полностью останавливает backend и освобождает guard. В текущем менеджере такого обработчика нет.

Нужно выбрать и проверить один порядок:

1. Зарезервировать удаление и сигнализировать отмену.
2. Инициировать остановку нужного backend, **без удаления payload**.
3. Дождаться всех выполняющих работу задач и освобождения ресурсов, включая owner.
4. Прочитать актуальное состояние и выполнить разрешённую очистку.

Либо использовать supervisor, который после сигнала самостоятельно выполняет пункты 2–3, а удалитель ждёт его результат. В обоих вариантах ожидание не должно зависеть от действия, поставленного после этого ожидания.

В текущем `spawn_add_torrent_task_inner` и `spawn_polling_task` JoinHandle отбрасываются. Одной записи guard в `TorrentRecord` недостаточно, чтобы доказать завершение этих задач. Сейчас `TorrentRecord` также имеет `#[derive(Clone)]`, а polling клонирует записи: non-Clone owner guard нельзя просто добавить в существующую структуру. Разделить снимок состояния и владение runtime; клонирование снимка не должно создавать или удерживать самостоятельного владельца операции.

Минимальный дополнительный тест: удаление активного торрента действительно достигает backend-stop и завершается, а не истекает по таймауту в `wait_owner`.

## A2. Stop и delete payload должны быть разными фазами

`TorrentManager::cancel_download` сейчас вызывает `engine.delete(id, true)`, а при отсутствии engine ID пытается удалить save_dir. Поэтому его нельзя использовать как недеструктивную фазу stop.

`stop_and_wait` должен иметь явно описанную семантику сохранения payload. В частности, `remove_download(..., false)` не должен удалять пользовательские файлы при остановке активного торрента. У существующего `engine.delete(id, false)` payload сохраняется, но fastresume-файлы всё равно удаляются: это тоже нужно учесть в договорённости о сохранении возможности продолжения.

Ещё одно отличие: `engine.delete` возвращает `Ok(())` при `PolicyRefused`, сохраняя файлы и снимая запись движка. Это допустимо только как явно сообщённый результат «запись удалена, файлы сохранены по политике». Оно не означает «все запрошенные файлы удалены».

Фиксированная задержка 50 мс после kill не является подтверждением завершения процесса. Для нового контракта нужны наблюдаемый результат остановки/завершения и гарантии конкретного process API, а не увеличение задержки.

## A3. Attempt должен защищать все отложенные изменения

Условный UPDATE `torrent_downloads` с `attempt_id` внутри транзакции — правильная замена upsert в завершении. Создание `downloads` при первом завершении сохраняется.

Но одной защиты completed/error недостаточно. Сейчас эти методы обновляют запись только по `game_id`:

- `update_torrent_progress`;
- `set_torrent_status`;
- `update_torrent_handle`.

Старые add/polling callbacks могут обновить новую попытку B после удаления A. Каждая запись, принадлежащая worker, должна проверять поколение и допустимый переход состояния. Нельзя дать старому progress сбросить `cancelling` обратно в `downloading` даже в пределах той же попытки.

Особенно проверить `record_completed_asset_download`: он сначала записывает `asset_installation = ready`, а уже потом вызывает завершающий метод downloads. Если проверять attempt только во втором вызове, запоздалая установка asset всё равно сохранится. Проверка и связанные записи должны входить в одну транзакцию.

Проверить также побочные задачи: после completed запускается `spawn_metadata_scrape`. Если результат удаления обещает удалить scrape-артефакты, поздний scraper не должен тут же создать их снова. Это следует явно включить в контракт либо ограничить обещание удаления состоянием загрузки.

Для UI отдельно решить обработку запоздавших IPC-событий. В текущем `DownloadProgressEvent` нет attempt, а store создаёт строку по пришедшему событию. Без фильтра устаревших поколений БД может оставаться корректной, но интерфейс на время покажет старую загрузку. Это не тот же дефект, что восстановление записи SQLite; тестировать их отдельно.

## A4. Очередь, пауза и восстановление не должны обходить координатор

Старт — это не только пользовательская команда. В текущем менеджере есть `trigger_next_queued`, resume и восстановление после запуска приложения. Все эти пути должны соблюдать резервирование удаления и правила attempt.

Нужно определить отдельно:

- queued-попытку, которая ещё не создала sidecar;
- paused-попытку, у которой нет активного чтения, но есть сохранённое состояние;
- resume: продолжение той же логической попытки или новая runtime-generation;
- startup restore строк без `attempt_id` после миграции.

Нельзя хранить Active-слот после паузы и затем безусловно вызывать `start()` при resume: это заблокирует законное продолжение собственной загрузки. Аналогично автостарт очереди не должен повторно резервировать уже принадлежащую ей попытку.

Миграция v3 соответствует текущему `user_version = 2`, но политика legacy NULL и генерации ID после перезапуска должна быть определена. Счётчик процесса пригоден только при гарантии нового пространства идентификаторов между запусками.

## A5. Точка регистрации должна предшествовать побочным эффектам

Дополнение второго рецензента принимается. В текущем HTTP-пути `preflight_disk_space` создаёт папку и при ошибке пишет failed-состояние **до** `direct_downloads.start`. `resolve_target_dir_override` тоже создаёт каталог до входа в основной поток.

При введении координатора проверить не только место старта скачивания, но и preflight, создание папок, failed-записи, hash/copy и распаковку. Чистую валидацию входа можно выполнить раньше; побочные эффекты предметной операции должны выполняться под её правом владения.

Completion из Drop доказывает завершение только тех ресурсов, которые действительно принадлежат этому owner. Рехэш и распаковка должны оставаться учитываемыми до фактического выхода блокирующей задачи, включая уничтожение/отмену родительского future. Отброшенный JoinHandle не доказывает остановку `spawn_blocking`.

Для `watch` следует различать явный сигнал завершения и неожиданное закрытие канала. Не трактовать закрытие как подтверждение чистой остановки, если канал мог закрыться до освобождения всех внешних ресурсов. Зафиксировать этот инвариант в устройстве owner и тестах.

## A6. Проблема удалителя подтверждается, но library_root недостаточно

`remove_recorded_download_path` действительно отличается от `safe_to_delete`: каноникализирует путь, запрещает только filesystem root и удаляет результат. Проверки принадлежности приложению и предков-reparse здесь нет.

Но простое требование «все пути внутри library_root» нарушит поддерживаемый custom target. В HTTP-ветке выбранный `target_dir` используется непосредственно, а для торрента создаётся дочерняя папка с ID игры. Требуется проверять **владение конкретными установленными файлами**, а не только общий корень.

Дополнительный риск по коду:

1. HTTP-архив записывается непосредственно в выбранную пользователем папку.
2. `extract_game_archive_if_needed` распаковывает его туда же.
3. Если подходящий игровой файл не найден, функция возвращает `Ok(extract_dir)`.
4. Этот путь может быть сохранён как completed `local_path`.
5. Удалитель рекурсивно удаляет записанную папку, где могли находиться посторонние файлы.

Этот сценарий пока не воспроизведён; он требует безопасного регрессионного теста в temp. Включить sentinel-файл, созданный до установки, и проверить его сохранность. Предпочтительна отдельная управляемая папка игры внутри выбранного назначения или manifest точных файлов установки; при неизвестном владении — отказ от рекурсивной очистки, а не расширение прав на всю выбранную папку.

Per-ID координатор также не предотвращает столкновение разных игр с одинаковым filename в одной custom-папке. Изолированное назначение/staging устраняет этот класс конфликтов; если сохраняется плоское назначение, нужна проверка владения путём.

Retry удаления при sharing violation полезен, но число попыток не заменяет проверку владения и остановку процесса. Retry должен быть ограниченным и различать временную блокировку и нарушение политики.

Файловая система и SQLite не образуют общую транзакцию. Если часть файлов удалена, а следующий файл не удалился, нельзя обещать полный откат диска. Очистка должна быть повторяемой; оставшееся состояние сохраняет информацию для повторной попытки. Обязательство «при ошибке stop нет разрушительной очистки» отличается от частичной ошибки уже начавшегося cleanup.

## B. Сетевой ответ второму рецензенту

Принимаются: ручная ограниченная цепочка редиректов, `Url::join`, проверка каждого перехода, pinned DNS для прямого соединения, лимиты фактически прочитанных байтов и общего бюджета времени.

Дополнительные требования:

- URL остаётся с исходным hostname. Для pinning использовать resolver/адреса подключения; не подменять URL на IP с последующим отключением TLS-проверки.
- На cross-origin переходах не пересылать чувствительные заголовки, cookies и credentials. Учесть credentials в самом URL; для внешних источников предпочтительно отклонять URL с userinfo.
- Ограничение ответа проверять при фактическом чтении; один `Content-Length` недостаточен.
- Ошибка или пустой результат DNS в режиме строгого прямого соединения не должны незаметно возвращать клиент к непроверенному повторному DNS-резолвингу. Сейчас `resolve_and_pin_host` допускает такой fallback ради proxy-совместимости.
- API провайдеров и внешние каталоги/изображения имеют разные политики и контексты credentials.

Предлагаемая фиксация proxy-policy для следующего патча: сохранять поддержку выбранного пользователем системного прокси и Fake-IP, явно считать прокси доверенным сетевым посредником. Гарантии проверенного конечного IP заявляются только для прямого режима с контролируемым подключением. В proxy-compatible режиме сохраняются проверки URL, редиректов, времени и объёма, но удалённый DNS/маршрутизация прокси остаются за его границей доверия. Не называть этот режим полной SSRF-изоляцией.

Это предлагаемая граница контракта, а не уже реализованная настройка. Сейчас нет подтверждённого отдельного переключателя режимов в продукте. Если требуется блокировать внутренние назначения даже за произвольным прокси, нужен другой явно согласованный контракт; молча отключать прокси нельзя.

По изображениям: в текущем Rust-пути выполняется кэширование байтов, серверного декодера изображений здесь нет. Лимит загрузки включить в патч. Ограничение размеров декодирования проверять там, где реально выполняется декодирование/отображение; не добавлять новый image decoder только ради формального выполнения рекомендации.

## Критерии согласования перед реализацией

Первому рецензенту: подтвердить исправленный порядок stop/join без тупика, семантику `delete_files=false`, покрытие всех worker-записей attempt, queue/resume/restore и правило владения custom-папками. Ниже приложен запрошенный код.

Второму рецензенту: подтвердить proxy-policy с ограниченными гарантиями конечного IP, fail-closed DNS в прямом режиме, cross-origin credentials и лимиты фактического чтения.

Для реализации нужны тесты A→удаление→B→поздний callback A, active/queued/paused torrent stop, сохранение payload при `delete_files=false`, зависшие headers/body, ошибка stop без cleanup, custom target с sentinel и migration/restore. Network-тесты проверяют разрешённые и запрещённые переходы, размер chunked-ответа и trust parity.

Порядок первого рецензента корректируется: примитивы coordinator с тестами; coherent-подключение прямых загрузок и conditional persistence; torrent runtime/stop/join; обработчик удаления и ownership-cleanup. Каждый промежуточный коммит должен компилироваться и проходить относящиеся к нему проверки. Не подключать новый удалитель до реализации необходимых stop-гарантий.

После объединения патчей: единый QA-прогон и отдельное Rust/Tauri-review на одном зафиксированном состоянии. Публикация/релиз не выполняются в рамках согласования.

## Приложение: запрошенный текущий код

Выдержки ниже взяты из HEAD, указанного в начале. Это исходный код для рецензирования, не предлагаемые исправления.

### TorrentRecord: текущий клонируемый снимок

src-tauri/src/torrent.rs, строки 32-49

```rust
struct TorrentRecord {
    game_id: String,
    save_dir: String,
    torrent_id: Option<usize>,
    state: TorrentRecordState,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TorrentRecordState {
    Queued,
    Resolving,
    Downloading,
    Paused,
    Completed,
    Cancelling,
    Error,
}
```

### TorrentManager::cancel_download

src-tauri/src/torrent.rs, строки 359-418

```rust
    pub async fn cancel_download(&self, game_id: String) -> Result<TorrentDownloadRecord, String> {
        let record = lock_store(&self.store)?
            .get_torrent_download(&game_id)?
            .ok_or_else(|| format!("Unknown torrent download: {game_id}"))?;

        if record.status == "completed" {
            return Err("Completed downloads cannot be cancelled.".to_string());
        }
        if is_direct_download_record(&record) && record.status == "downloading" {
            return Err(
                "Direct downloads cannot be cancelled while transfer is active.".to_string(),
            );
        }

        let cancelling =
            lock_store(&self.store)?.set_torrent_status(&game_id, "cancelling", None)?;
        emit_download_record(&self.app, &cancelling);

        let runtime_id = {
            let mut records = self.records.lock().await;
            if let Some(record) = records.get_mut(&game_id) {
                record.state = TorrentRecordState::Cancelling;
                record.torrent_id
            } else {
                None
            }
        };

        let torrent_id = runtime_id.or_else(|| record.torrent_id.and_then(i64_to_usize));
        if let Some(id) = torrent_id {
            if let Err(error) = self.engine.delete(id, true).await {
                let record =
                    lock_store(&self.store)?.set_torrent_status(&game_id, "error", Some(&error))?;
                set_runtime_error(&self.records, &game_id, error).await;
                emit_download_record(&self.app, &record);
                return Ok(record);
            }
        } else if let Err(cleanup_error) =
            remove_save_dir_if_safe(&self.data_dir, &record.save_dir).await
        {
            let record = lock_store(&self.store)?.set_torrent_status(
                &game_id,
                "error",
                Some(&cleanup_error),
            )?;
            set_runtime_error(&self.records, &game_id, cleanup_error).await;
            emit_download_record(&self.app, &record);
            return Ok(record);
        }

        {
            let mut records = self.records.lock().await;
            records.remove(&game_id);
        }

        let cancelled = lock_store(&self.store)?.set_torrent_status(&game_id, "cancelled", None)?;
        emit_download_record(&self.app, &cancelled);
        self.process_next_queued_download().await;
        Ok(cancelled)
    }
```

### Автостарт очереди

src-tauri/src/torrent.rs, строки 566-631

```rust
async fn trigger_next_queued(
    engine: Arc<dyn TorrentEngine>,
    records: Arc<Mutex<HashMap<String, TorrentRecord>>>,
    store: Arc<StoreMutex<RepositoryStore>>,
    app: AppHandle,
) {
    let next_task = {
        let records_guard = records.lock().await;
        if active_download_count(&records_guard) >= MAX_ACTIVE_DOWNLOADS {
            return;
        }

        let in_memory_id = records_guard
            .values()
            .find(|r| r.state == TorrentRecordState::Queued)
            .map(|r| r.game_id.clone());

        if let Some(game_id) = in_memory_id {
            let store_guard = match lock_store(&store) {
                Ok(s) => s,
                Err(_) => return,
            };
            store_guard
                .get_torrent_download(&game_id)
                .ok()
                .flatten()
                .map(|r| (r.game_id, r.magnet_uri, r.save_dir))
        } else {
            let store_guard = match lock_store(&store) {
                Ok(s) => s,
                Err(_) => return,
            };
            store_guard
                .get_next_queued_torrent_download()
                .ok()
                .flatten()
                .map(|r| (r.game_id, r.magnet_uri, r.save_dir))
        }
    };

    let Some((game_id, magnet_uri, save_dir)) = next_task else {
        return;
    };

    if let Ok(record) =
        lock_store(&store).and_then(|s| s.set_torrent_status(&game_id, "resolving", None))
    {
        emit_download_record(&app, &record);
    }

    {
        let mut records_guard = records.lock().await;
        records_guard.insert(
            game_id.clone(),
            TorrentRecord {
                game_id: game_id.clone(),
                save_dir: save_dir.clone(),
                torrent_id: None,
                state: TorrentRecordState::Resolving,
                error: None,
            },
        );
    }

    spawn_add_torrent_task_inner(engine, records, store, app, game_id, magnet_uri, save_dir);
}
```

### Фоновая задача добавления торрента

src-tauri/src/torrent.rs, строки 633-707

```rust
fn spawn_add_torrent_task_inner(
    engine: Arc<dyn TorrentEngine>,
    records: Arc<Mutex<HashMap<String, TorrentRecord>>>,
    store: Arc<StoreMutex<RepositoryStore>>,
    app: AppHandle,
    game_id: String,
    magnet_uri: String,
    save_dir: String,
) {
    tauri::async_runtime::spawn(async move {
        let add_result = engine.add_magnet(magnet_uri, save_dir).await;

        match add_result {
            Ok(id) => {
                let desired_state = {
                    let mut records_guard = records.lock().await;
                    records_guard.get_mut(&game_id).map(|record| {
                        record.torrent_id = Some(id);
                        record.error = None;
                        match record.state {
                            TorrentRecordState::Paused => TorrentRecordState::Paused,
                            TorrentRecordState::Cancelling => record.state,
                            _ => {
                                record.state = TorrentRecordState::Downloading;
                                TorrentRecordState::Downloading
                            }
                        }
                    })
                };

                match desired_state {
                    Some(TorrentRecordState::Paused) => {
                        let _ = engine.pause(id).await;
                        if let Ok(record) = lock_store(&store).and_then(|store| {
                            store.update_torrent_handle(&game_id, id as i64, "paused")
                        }) {
                            emit_download_record(&app, &record);
                        }
                    }
                    Some(TorrentRecordState::Cancelling) => {
                        let _ = engine.delete(id, true).await;
                        if let Ok(record) = lock_store(&store)
                            .and_then(|store| store.set_torrent_status(&game_id, "cancelled", None))
                        {
                            emit_download_record(&app, &record);
                        }
                    }
                    Some(_) => {
                        let _ = engine.resume(id).await;
                        if let Ok(record) = lock_store(&store).and_then(|store| {
                            store.update_torrent_handle(&game_id, id as i64, "downloading")
                        }) {
                            emit_download_record(&app, &record);
                        }
                        spawn_polling_task(engine, records, store, app, game_id, id);
                    }
                    None => {
                        let _ = engine.delete(id, true).await;
                    }
                }
            }
            Err(error) => {
                set_torrent_error(
                    &records,
                    &store,
                    &app,
                    &game_id,
                    format!("Failed to start magnet download: {error}"),
                )
                .await;
                trigger_next_queued(engine, records, store, app).await;
            }
        }
    });
}
```

### Polling и завершающие записи

src-tauri/src/torrent.rs, строки 988-1101

```rust
fn spawn_polling_task(
    engine: Arc<dyn TorrentEngine>,
    records: Arc<Mutex<HashMap<String, TorrentRecord>>>,
    store: Arc<StoreMutex<RepositoryStore>>,
    app: AppHandle,
    game_id: String,
    torrent_id: usize,
) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        let mut last_flush = Instant::now() - DB_FLUSH_INTERVAL;
        let mut last_status = String::new();

        loop {
            interval.tick().await;

            let record = {
                let records = records.lock().await;
                records.get(&game_id).cloned()
            };

            let Some(record) = record else {
                break;
            };

            if matches!(
                record.state,
                TorrentRecordState::Paused
                    | TorrentRecordState::Cancelling
                    | TorrentRecordState::Completed
                    | TorrentRecordState::Error
            ) {
                break;
            }

            let Some(stats) = engine.stats(torrent_id) else {
                break;
            };
            let status = status_from_stats(&record, &stats);
            let event = event_from_status(&status);
            let _ = app.emit("download:progress", event);

            let status_changed = status.state != last_status;
            let should_flush = last_flush.elapsed() >= DB_FLUSH_INTERVAL
                || status_changed
                || status.finished
                || status.error.is_some();

            if should_flush {
                last_status = status.state.clone();
                last_flush = Instant::now();

                let persisted = if status.finished {
                    lock_store(&store).and_then(|store| {
                        store.mark_torrent_completed(
                            &game_id,
                            &status.save_dir,
                            status.downloaded_bytes,
                            status.total_bytes,
                        )
                    })
                } else {
                    lock_store(&store).and_then(|store| {
                        store.update_torrent_progress(
                            &game_id,
                            &status.state,
                            status.progress * 100.0,
                            status.downloaded_bytes,
                            status.total_bytes,
                            status.download_speed_bytes_per_sec,
                            status.upload_speed_bytes_per_sec,
                            status.peers_count,
                        )
                    })
                };

                if let Ok(record) = persisted {
                    emit_download_record(&app, &record);
                }
            }

            if let Some(error) = status.error {
                set_torrent_error(&records, &store, &app, &game_id, error).await;
                trigger_next_queued(
                    Arc::clone(&engine),
                    Arc::clone(&records),
                    Arc::clone(&store),
                    app.clone(),
                )
                .await;
                break;
            }

            if status.finished {
                {
                    let mut records = records.lock().await;
                    if let Some(record) = records.get_mut(&game_id) {
                        record.state = TorrentRecordState::Completed;
                        record.error = None;
                    }
                }
                let _ = engine.delete(torrent_id, false).await;
                trigger_next_queued(
                    Arc::clone(&engine),
                    Arc::clone(&records),
                    Arc::clone(&store),
                    app.clone(),
                )
                .await;
                break;
            }
        }
    });
}
```

### LibtorrentEngine::delete

src-tauri/src/libtorrent_engine.rs, строки 657-780

```rust
    async fn delete(&self, id: usize, delete_files: bool) -> Result<(), String> {
        // 1. Signal stop to child process and mark deleting = true
        {
            let mut map = self.lock()?;
            let Some(entry) = map.get_mut(&id) else {
                return Ok(());
            };
            entry.deleting = true;
            entry.user_stopped = true;
            if let Some(child) = entry.child.as_mut() {
                let _ = child.write(b"{\"action\":\"stop\"}\n");
            }
        }

        // 2. Wait up to 5s (100 * 50ms) for spawning to finish and child process to exit cleanly
        for _ in 0..100 {
            let is_dead = {
                let map = self.lock()?;
                map.get(&id)
                    .is_none_or(|e| !e.spawning && e.child.is_none())
            };
            if is_dead {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // 3. If child is still present, kill it. If kill fails, reset deleting and return error
        let child_to_kill = {
            let mut map = self.lock()?;
            map.get_mut(&id).and_then(|e| e.child.take())
        };
        if let Some(child) = child_to_kill {
            if let Err(e) = child.kill() {
                if let Ok(mut map) = self.lock() {
                    if let Some(entry) = map.get_mut(&id) {
                        entry.deleting = false;
                        entry.stats.error =
                            Some(format!("Failed to kill torrent process during delete: {e}"));
                    }
                }
                return Err(format!("Failed to kill torrent process during delete: {e}"));
            }
        }

        // 4. Brief delay to allow Windows file locks to be fully released
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // 5. Get entry info WITHOUT removing from map yet!
        let (magnet, save_dir) = {
            let map = self.lock()?;
            let Some(entry) = map.get(&id) else {
                return Ok(());
            };
            (entry.magnet.clone(), entry.save_dir.clone())
        };

        // 6. If delete_files, validate and delete files first
        if delete_files {
            let state = self.app.try_state::<crate::AppState>().ok_or_else(|| {
                if let Ok(mut map) = self.lock() {
                    if let Some(e) = map.get_mut(&id) {
                        e.deleting = false;
                    }
                }
                "Application state is not initialized for torrent file deletion.".to_string()
            })?;
            let library_root = crate::commands::library_root_for_app_state(&state);
            match safe_to_delete(std::path::Path::new(&save_dir), &library_root) {
                SafeDeleteOutcome::Safe(path) => {
                    if path.exists() {
                        if let Err(e) = std::fs::remove_dir_all(&path) {
                            if let Ok(mut map) = self.lock() {
                                if let Some(entry) = map.get_mut(&id) {
                                    entry.deleting = false;
                                    entry.stats.error =
                                        Some(format!("Failed to delete files: {e}"));
                                }
                            }
                            return Err(format!(
                                "Failed to delete torrent directory {}: {e}",
                                path.display()
                            ));
                        }
                    }
                }
                SafeDeleteOutcome::SafeJunction(path) => {
                    if path.exists() || is_reparse_point_or_symlink(&path) {
                        if let Err(e) = std::fs::remove_dir(&path) {
                            if let Ok(mut map) = self.lock() {
                                if let Some(entry) = map.get_mut(&id) {
                                    entry.deleting = false;
                                    entry.stats.error =
                                        Some(format!("Failed to unlink junction: {e}"));
                                }
                            }
                            return Err(format!(
                                "Failed to unlink junction {}: {e}",
                                path.display()
                            ));
                        }
                    }
                }
                SafeDeleteOutcome::Nonexistent => {}
                SafeDeleteOutcome::PolicyRefused(reason) => {
                    // Files outside library root or in forbidden directories are preserved.
                    // DO NOT block removal of the launcher record so user is not stuck.
                    eprintln!("Torrent delete files policy refusal (files preserved): {reason}");
                }
            }
        }

        // 7. Delete fastresume files
        if let Some(btih) = extract_btih(&magnet) {
            let save_path = std::path::Path::new(&save_dir);
            let _ = std::fs::remove_file(save_path.join(format!(".{btih}.fastresume")));
            let _ = std::fs::remove_file(save_path.join(format!(".{btih}.fastresume.tmp")));
        }

        // 8. ONLY remove entry after all deletion operations succeeded!
        self.lock()?.remove(&id);

        Ok(())
    }
```

### Текущий удалитель записанного пути

src-tauri/src/commands/diagnostics_helpers.rs, строки 310-335

```rust
pub(super) fn remove_recorded_download_path(candidate: &Path) -> Result<(), String> {
    if !candidate.exists() {
        return Ok(());
    }
    let canonical_candidate = fs::canonicalize(candidate)
        .map_err(|error| format!("Failed to inspect {}: {error}", candidate.display()))?;
    if canonical_candidate.file_name().is_none() {
        return Err(format!(
            "Refusing to delete filesystem root: {}",
            canonical_candidate.display()
        ));
    }
    if canonical_candidate.is_dir() {
        fs::remove_dir_all(&canonical_candidate)
    } else {
        fs::remove_file(&canonical_candidate)
    }
    .map_err(|error| {
        format!(
            "Failed to remove {}: {error}",
            canonical_candidate.display()
        )
    })
}

pub(super) fn open_path(path: &Path) -> Result<(), String> {
```

### HTTP custom destination

src-tauri/src/commands/downloads.rs, строки 737-748

```rust
fn game_direct_download_destination(
    default_root: PathBuf,
    target_dir: Option<&Path>,
    game: &CatalogGameView,
    source: &SourceUri,
) -> PathBuf {
    if let Some(target_dir) = target_dir {
        return target_dir.join(file_name_for_source(source, &game.title));
    }
    destination_for_source(&default_root, &game.platform, &game.id, source, &game.title)
}

```

### Распаковка и fallback на родительскую папку

src-tauri/src/commands/downloads.rs, строки 312-355

```rust
pub(super) fn extract_game_archive_if_needed(
    game: &CatalogGameView,
    downloaded_path: &Path,
) -> Result<PathBuf, String> {
    let extension = downloaded_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();

    if !extension.eq_ignore_ascii_case("zip") && !extension.eq_ignore_ascii_case("7z") {
        return Ok(downloaded_path.to_path_buf());
    }

    if game.expected_extensions.iter().any(|ext| {
        ext.eq_ignore_ascii_case(".zip")
            || ext.eq_ignore_ascii_case("zip")
            || ext.eq_ignore_ascii_case(".7z")
            || ext.eq_ignore_ascii_case("7z")
    }) {
        return Ok(downloaded_path.to_path_buf());
    }

    let extract_dir = downloaded_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("Game archive has no parent: {}", downloaded_path.display()))?;

    crate::archive::extract_archive_safely(downloaded_path, &extract_dir)?;
    let _ = fs::remove_file(downloaded_path);

    if let Ok(normalized_extensions) =
        crate::game_files::normalize_expected_extensions(&game.expected_extensions)
    {
        if let Ok(resolved) = crate::game_files::resolve_game_path(
            &extract_dir,
            &normalized_extensions,
            game.launch.as_ref().and_then(|launch| launch.preferred_file.as_deref()),
        ) {
            return Ok(resolved);
        }
    }

    Ok(extract_dir)
}
```

### Завершение asset перед завершающей транзакцией downloads

src-tauri/src/commands/downloads.rs, строки 228-254

```rust
fn record_completed_asset_download(
    state: &AppState,
    app: &AppHandle,
    asset: &AssetView,
    source_kind: &str,
    local_path: &Path,
    sha256: Option<&str>,
    total_bytes: u64,
) -> Result<DownloadRecord, String> {
    let local_path = local_path.to_string_lossy().to_string();
    let store = lock_app_store(state)?;
    store.record_asset_installation(&asset.id, Some(&local_path), "ready", sha256, None)?;
    if let Some(sha256) = sha256 {
        let (record, torrent) = store.record_direct_asset_download_completed(
            &asset.id,
            source_kind,
            &local_path,
            sha256,
            total_bytes,
        )?;
        drop(store);
        emit_direct_download_record(app, &torrent);
        Ok(record)
    } else {
        store.record_download(&asset.id, "asset", Some(&local_path), None, None)
    }
}
```
