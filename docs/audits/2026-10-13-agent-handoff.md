# Fusion Launcher: пакет сверки для двух внешних ИИ-рецензентов

Дата: 13 октября 2026 года. Задача этого раунда — согласовать разбор и требования к исправлениям. Патчи приложения не выполнялись.

## Зафиксированная версия и границы проверки

- HEAD: `2ff0ec4505fe463583678609b0fd8efe061ba727` (`2ff0ec4`, `fix(deep-link): prevent modal fetch race conditions, add i18n and clear emulator notice`).
- На старте сверки рабочее дерево было чистым. Этот документ — единственный намеренно добавленный файл.
- Оба присланных разбора рассматриваются как независимые рецензии, а примеры кода — как предложения, требующие проверки совместимости.
- Выводы ниже получены чтением локального кода и результатами локальных проверок. Эксплуатация SSRF и гонок удаления на работающем приложении в этом раунде не воспроизводилась.
- Нет оснований считать расхождение 202/205 доказанной регрессией. Для сравнения нужны SHA и команды из второго отчёта.

## Общий вывод и приоритеты

Оба разбора верно выделяют сетевые ограничения и жизненный цикл загрузок. До релиза следует исправить оба блока; порядок их независимой реализации можно разделить между исполнителями.

Обозначение P0 пока не подтверждено последствиями и воспроизведением. Для планирования предлагается P1 для обхода сетевой политики и удаления активной загрузки, P2 для задержек библиотеки, P3 для разделения React-компонентов. Отмена зависшей передачи входит в обязательный P1-блок загрузок. Несогласованное отображение доверия также исправляется до релиза, но само по себе не доказывает выполнение чужого кода.

| Блок | Что подтверждено | Критерий результата |
| --- | --- | --- |
| Загрузки | Удаление HTTP-загрузки вызывает торрент-менеджер и игнорирует ошибку | После успешного удаления нет активной задачи, записи и файлов не появляются повторно |
| Отмена | `chunk().await` не прерывается флагом; задан только connect timeout | Отмена работает при зависании до заголовков и во время чтения тела |
| Сеть | Загрузчики repository/manifest используют стандартные редиректы | Каждый реально выполняемый переход соответствует принятой сетевой политике |
| Доверие | Preview использует объявленный `official`, storage понижает его | Предпросмотр и сохранение возвращают один эффективный уровень |
| Масштабирование | `get_library_statuses` удерживает общий store lock на весь обход | Сначала измерения, затем отсутствие дискового обхода под глобальным lock |

## Поправки к предложениям первого рецензента

### 1. Нельзя просто заменить завершение на UPDATE downloads

`record_direct_game_download_started` создаёт или обновляет строку в `torrent_downloads`. Строка в `downloads` при первом скачивании появляется при завершении, через `record_download`.

Поэтому предложенное `UPDATE downloads ... WHERE subject_id = ?` даст ноль обновлений на обычной первой установке. Это сломает корректный сценарий, а не только заблокирует восстановление удалённой записи.

Защиту от запоздалой записи следует привязать к существующей активной операции и, если вводится идентификатор поколения, к этому поколению. Сохранение результата должно атомарно проверять, что именно эта операция ещё имеет право завершиться. Проверка одного `subject_id` не различает старую и новую попытки скачивания.

При этом первичная защита — остановка и ожидание владельца операции перед очисткой. Нельзя ограничиваться изменением одного SQL-запроса: отдельно существуют записи assets, failed/completed и post-processing.

### 2. cancel_and_wait нужен вместе с запретом нового старта во время удаления

В текущем `DirectDownloadRegistry` есть флаг отмены и guard, удаляющий запись из реестра в `Drop`. Сигнала завершения для ожидающего обработчика нет.

Требования к исправлению:

1. Зарезервировать удаление данного subject: новая загрузка с тем же ID не может стартовать до окончания очистки.
2. Сигнализировать отмену активному владельцу без удержания SQLite mutex.
3. Дождаться освобождения всех ресурсов владельца операции: HTTP response, открытого файла, возобновляющего хеширования, распаковки и завершающих записей.
4. При таймауте или ошибке остановки вернуть ошибку и сохранить состояние для повторной попытки. Не начинать очистку.
5. После остановки заново прочитать актуальные записи и пути, затем очистить диск и БД.
6. Снять резервирование удаления после завершения очистки.

Последовательные `cancel_and_wait(direct)` и `cancel_and_wait(torrent)` сами по себе не закрывают промежуток, в котором может стартовать новая операция. Нужен общий протокол удаления/старта для ID.

В `TorrentManager` метода `cancel_and_wait` сейчас нет. Его нельзя считать готовым API. Нужно проверить завершение фоновой задачи добавления, polling и sidecar, а также возвращённый статус: `cancel_download` в некоторых ветках возвращает `Ok(record)` со статусом `error`.

### 3. Есть второй путь прямого скачивания, обходящий реестр

`commands/downloads.rs::download_game` всё ещё зарегистрирован как Tauri-команда в `lib.rs` и доступен через `api.downloadGame`. Он вызывает `download_source_to_file` без `DirectDownloadRegistry` и без отмены. Исправление только `start_game_download_internal` оставит доступный обход нового протокола.

Этот путь нужно перевести на общую реализацию или удалить после проверки совместимости. Аналогично проверить скачивание assets и связанный post-processing.

### 4. Отмена должна охватывать headers и body; partial-файлы имеют разные правила

`select!` вокруг `response.chunk()` исправляет зависание тела, но ожидание `request.send()` тоже должно прерываться. Следует учесть DNS, возобновляющее хеширование и распаковку: уже запущенный `spawn_blocking` не прекращается от простого уничтожения ожидающего future.

Нельзя безусловно удалять `.part` при любой сетевой ошибке: текущая реализация поддерживает возобновление. Удаление partial при явной отмене/удалении и сохранение для повторной попытки при обрыве — разные решения. Политику нужно зафиксировать и проверить. Дескриптор перед удалением должен быть закрыт, особенно на Windows.

Общий таймаут всей загрузки нежелателен для больших архивов. Таймаут бездействия и сигнал отмены решают другую задачу и не должны ограничивать длительность нормально продвигающейся загрузки.

### 5. Значение untrusted отсутствует в текущей схеме

Rust `validate_trust_level` и TypeScript `RepositoryTrustLevel` допускают только `official`, `community`, `unknown`. Предложенный helper с `untrusted` несовместим с контрактом.

Утверждение о пропуске произвольных `verified`/`trusted` через обычный repository preview тоже не подтверждается: `fetch_repository_schema` вызывает `validate_repository_schema`, а локальный JSON проходит ту же проверку. Manifest-конвертация задаёт `unknown` самостоятельно.

Реальная проблема остаётся: допустимое значение `official` приходит из недоверенного JSON в preview, тогда как storage его понижает. Нужен общий `effective_trust_level`, сохраняющий существующий набор значений. Для встроенного источника — `official`, для внешнего объявленного `official` — `community`, остальные допустимые значения согласованно сохраняются.

## Согласование сетевого исправления

`https_only_redirect_policy` проверяет схему, строку host и число переходов. Она не проверяет DNS-ответ нового hostname. Подключение только этой функции к двум клиентам — полезный, но неполный патч.

Допустимы два явно оформленных варианта:

- Отключить автоматические редиректы у внешних JSON-источников и возвращать понятную ошибку с требованием прямого URL. Это закрывает данный обход, но меняет совместимость существующих источников.
- Выполнять ограниченную цепочку вручную: отключить автоматические редиректы, разрешать `Location` относительно текущего URL, проверять каждый URL и адрес назначения, выполнять запрос с проверенным DNS-результатом. Нужны общий бюджет времени, ограничение числа переходов и размера ответа.

Для сохранения совместимости предпочтителен второй вариант после тестов. Если выбирается временный запрет редиректов, изменение поведения должно быть явно описано пользователю.

Системный прокси и Fake-IP — отдельная граница гарантий. Текущий код намеренно допускает `198.18.0.0/15`, а при ошибке локального DNS разрешает reqwest продолжить запрос. Нельзя заявлять полную проверку конечного IP за прокси только на основании локального DNS pinning. Не отключать прокси и Fake-IP молча: сначала определить модель доверия и совместимость.

Общая фабрика HTTP-клиентов полезна, но должна различать внешние источники, большие загрузки, GitHub и API метаданных. У них разные timeout, redirect и size-policy. Запрет конструкторов через Clippy — дополнительная дисциплина, а не доказательство SSRF-защиты. Прямой builder внутри фабрики всё равно должен применять правильную политику, а lint необходимо проверить отрицательным примером.

Инвентаризация выявила ещё один путь: `scraper.rs::ensure_artwork_cached` принимает HTTP/HTTPS URL, создаёт клиент со стандартными редиректами и читает всё тело через `.bytes()` без ограничения размера. Он вызывается командой `cache_artwork` и фоновым scraper. Его необходимо включить в сетевой раунд. Это дополнительная находка по коду; сквозной сценарий с недоверенным URL следует отдельно воспроизвести.

ScreenScraper/SteamGridDB работают с фиксированными API; их тоже нужно учесть в фабрике, но не смешивать автоматически с внешними catalog URL и не менять без анализа передачу credentials.

## Обязательные регрессионные сценарии

Для блока загрузок:

- Удаление в середине HTTP-передачи: worker завершён до очистки, запись не восстанавливается после ожидания.
- Сервер завис до заголовков; сервер завис после первого чанка: отмена завершается за ограниченное время, файл можно удалить на Windows.
- Конкурентный старт того же ID во время удаления не обходит резервирование; новая явная установка после удаления работает.
- Отмена/удаление во время распаковки и непосредственно перед completed-записью.
- Таймаут ожидания остановки не приводит к удалению записей или файлов.
- Первое скачивание, повтор после ошибки и скачивание assets продолжают сохранять корректный completed-результат.
- Старый экспортированный `download_game` не обходит общий протокол.
- Обрыв сети сохраняет пригодный partial согласно принятой политике; отмена очищает его.
- Перезапуск после прерывания даёт корректное состояние и возможность повторить операцию.

Для сетевого блока:

- HTTPS → HTTP, loopback, private и link-local; новый hostname с запрещённым DNS-ответом; IPv4-mapped IPv6.
- Разрешённый относительный редирект, цикл, превышение лимита, отсутствие/повреждение `Location`.
- Превышение размера JSON/изображения, включая ответ без `Content-Length`.
- Debug-localhost остаётся отдельной явно ограниченной возможностью.
- Preview и storage совпадают для встроенного источника и внешних `official`, `community`, `unknown`.
- Неизвестные значения trust по-прежнему отклоняются валидатором.

Тесты должны проверять реальную Rust-ветку. Детерминированный локальный HTTP/TLS fixture или инъекция транспорта допустимы; нельзя ослаблять production-политику ради localhost-теста. Реальный сторонний сайт не должен быть обязательной зависимостью CI.

## Релизные проверки и интерпретация результатов

В предыдущем локальном анализе этого дерева прошли TypeScript, frontend build, static check, 202 Rust-теста, fmt, Clippy и 5 preview-smoke проверок. Из 88 Node-тестов прошли 87; один остановился на `EPERM` при записи `.tmp/source-library-template-pages-check/stale.txt`. Полный `npm run check` поэтому не был успешным.

В текущем раунде повторно выполнена инвентаризация тестов: `cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib -- --list` возвращает **202 tests, 0 benchmarks**. Это подтверждение числа тестов, а не новый полный QA-прогон.

Тест Pages artifact действительно использует фиксированную папку внутри репозитория, а builder в режиме `--check` жёстко использует тот же путь и очищает его. Для изоляции недостаточно заменить только тест на `mkdtemp`: builder должен принимать соответствующий output path. Причина конкретного `EPERM` пока не установлена; конкурентные запуски также требуют отдельной проверки.

`mvp:smoke` напрямую импортирует `previewApi` и проверяет его. Это не проверка IPC, реального downloader или запуска процесса. `mvp:visual` также нельзя автоматически считать проверкой backend. В проекте уже существует отдельный packaged smoke; его надо изучить и расширить, а не утверждать, что проверок бинарника вообще нет.

Fake-эмулятор — подходящий следующий тест: через реальный Rust-путь сохранить путь, запустить fixture-игру, проверить argv, изменить exe и убедиться в отказе запуска. Один такой тест сам по себе не доказывает корректность дерева процессов и учёта времени: для этих свойств нужны отдельные проверки завершения и данных о сессии. `tauri::test::mock_app` потребует проверить feature/config и Windows-совместимость.

Генерация типов и фикстур может уменьшить расхождение Rust/previewData, но это отдельная архитектурная работа. Не включать tauri-specta и массовый рефакторинг в обязательный патч отмены.

## Предлагаемое разделение следующего раунда

- Первый внешний рецензент: проверить модель отмены/ожидания и SQL-инварианты по приложенным выдержкам; отдельно ответить, как закрывается повторный старт и legacy `download_game`.
- Второй внешний рецензент: проверить сетевую модель, допустимость ручных редиректов, поведение прокси/Fake-IP и границы релизных доказательств.
- Исполнитель: после согласования подготовить два независимых патча — lifecycle и network/trust; выполнить регрессионные тесты и QA на одном SHA. Изменения Rust после реализации проходят отдельное backend-review.

Производительность библиотеки и разделение крупных UI-компонентов остаются следующими задачами. Требования Visual OS применяются, если дальнейшие патчи затронут интерфейс.

## Приложение: текущие исходники

Ниже приведены выдержки из зафиксированного дерева, а не предлагаемые патчи. Номера строк относятся к указанному HEAD.

### DirectDownloadRegistry, токен и Drop guard

src-tauri/src/downloads.rs, строки 36-124

```rust
#[derive(Clone, Default)]
pub struct DirectDownloadRegistry {
    tokens: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

#[derive(Clone)]
pub struct DirectDownloadCancelToken {
    token: Arc<AtomicBool>,
}

pub struct DirectDownloadGuard {
    id: String,
    token: Arc<AtomicBool>,
    registry: DirectDownloadRegistry,
}

impl DirectDownloadRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self, id: &str) -> Result<DirectDownloadGuard, String> {
        let id = id.trim();
        if id.is_empty() {
            return Err("Download id cannot be empty.".to_string());
        }

        let token = Arc::new(AtomicBool::new(false));
        let mut tokens = self.lock_tokens()?;
        if tokens.contains_key(id) {
            return Err(format!("Download is already active: {id}"));
        }
        tokens.insert(id.to_string(), Arc::clone(&token));

        Ok(DirectDownloadGuard {
            id: id.to_string(),
            token,
            registry: self.clone(),
        })
    }

    pub fn cancel(&self, id: &str) -> Result<bool, String> {
        let Some(token) = self.lock_tokens()?.get(id).cloned() else {
            return Ok(false);
        };
        token.store(true, Ordering::SeqCst);
        Ok(true)
    }

    fn finish(&self, id: &str, token: &Arc<AtomicBool>) {
        let Ok(mut tokens) = self.tokens.lock() else {
            return;
        };
        if tokens
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, token))
        {
            tokens.remove(id);
        }
    }

    fn lock_tokens(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<String, Arc<AtomicBool>>>, String> {
        self.tokens
            .lock()
            .map_err(|_| "Direct download registry lock is poisoned.".to_string())
    }
}

impl DirectDownloadGuard {
    pub fn token(&self) -> DirectDownloadCancelToken {
        DirectDownloadCancelToken {
            token: Arc::clone(&self.token),
        }
    }
}

impl Drop for DirectDownloadGuard {
    fn drop(&mut self) {
        self.registry.finish(&self.id, &self.token);
    }
}

impl DirectDownloadCancelToken {
    pub fn is_cancelled(&self) -> bool {
        self.token.load(Ordering::SeqCst)
    }
}
```

### Обработчик удаления

src-tauri/src/commands/downloads.rs, строки 825-888

```rust
async fn remove_download_internal(
    download_id: &str,
    delete_files: bool,
    state: &AppState,
    event_name: &str,
) -> Result<bool, String> {
    let download = {
        let store = lock_app_store(state)?;
        (
            store.get_download(download_id)?,
            store.get_torrent_download(download_id)?,
        )
    };

    if let Some(torrent) = download.1.as_ref() {
        if !matches!(torrent.status.as_str(), "completed" | "cancelled") {
            if let Ok(torrents) = state.torrents() {
                let _ = torrents.cancel_download(download_id.to_string()).await;
            }
        }
    }

    if delete_files {
        let mut candidates = Vec::new();
        if let Some(record) = download.0.as_ref() {
            if let Some(path) = record.local_path.as_ref() {
                candidates.push(PathBuf::from(path));
            }
        }
        if let Some(torrent) = download.1.as_ref() {
            candidates.push(PathBuf::from(&torrent.save_dir));
        }
        for candidate in candidates {
            remove_recorded_download_path(&candidate)?;
        }
    }

    let store = lock_app_store(state)?;
    let is_asset = download
        .0
        .as_ref()
        .is_some_and(|record| record.subject_type == "asset")
        || download
            .1
            .as_ref()
            .and_then(|record| record.subject_type.as_deref())
            == Some("asset");
    let mut changed = download.0.is_some() || download.1.is_some();
    if is_asset {
        store.delete_asset_state(download_id)?;
        changed = store.delete_torrent_download(download_id)? || changed;
    } else {
        changed = store.delete_download(download_id)? || changed;
        changed = store.delete_torrent_download(download_id)? || changed;
        changed = store.delete_scrape_artifacts(download_id)? || changed;
    }
    logging::log_event(
        &state.data_dir,
        event_name,
        &[("download_id", download_id)],
    );
    Ok(changed)
}

```

### Ожидание headers и чтение HTTP body

src-tauri/src/downloads.rs, строки 370-458

```rust
    let mut builder = reqwest::Client::builder()
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .user_agent(DOWNLOAD_USER_AGENT)
        .redirect(redirect_policy);
    builder = crate::net::resolve_and_pin_host(builder, &parsed_url)
        .await
        .map_err(|error| format!("SSRF protection rejected download target: {error}"))?;
    let client = builder
        .build()
        .map_err(|error| format!("Failed to initialize downloader: {error}"))?;
    let mut request = client.get(url);
    if resume_offset > 0 {
        request = request.header(RANGE, format!("bytes={resume_offset}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Failed to fetch source: {error}"))?;

    // If we asked for a range but the server replied 200 OK, it ignored the
    // Range header and is sending the whole file again: restart from scratch.
    let mut append = false;
    if resume_offset > 0 {
        if response.status() == StatusCode::PARTIAL_CONTENT {
            append = true;
        } else if response.status().is_success() {
            hasher = Sha256::new();
            downloaded = 0;
            let _ = tokio::fs::remove_file(&part_path).await;
        }
    }
    let mut response = response
        .error_for_status()
        .map_err(|error| format!("Source returned an error: {error}"))?;

    // For a 206 resume the body is only the remaining bytes, so add what we
    // already have on disk to report a meaningful total.
    let total_bytes = response
        .content_length()
        .map(|len| len + resume_offset)
        .or(options.expected_size_bytes);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!append)
        .append(append)
        .open(&part_path)
        .await
        .map_err(|error| format!("Failed to open download file: {error}"))?;

    on_progress(downloaded, total_bytes);
    let mut last_report = Instant::now();

    loop {
        if is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(DOWNLOAD_CANCELLED_MESSAGE.to_string());
        }
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => return Err(format!("Failed to read source body: {error}")),
        };
        if is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(DOWNLOAD_CANCELLED_MESSAGE.to_string());
        }
        downloaded += chunk.len() as u64;
        if let Some(max_bytes) = options.max_bytes {
            if downloaded > max_bytes {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(format!(
                    "Download exceeded the size limit of {max_bytes} bytes."
                ));
            }
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("Failed to write download: {error}"))?;

        if last_report.elapsed() >= PROGRESS_REPORT_INTERVAL {
            on_progress(downloaded, total_bytes);
            last_report = Instant::now();
        }
```

### Начало прямой загрузки: таблица torrent_downloads

src-tauri/src/storage/downloads.rs, строки 536-579

```rust
    pub fn record_direct_game_download_started(
        &self,
        game_id: &str,
        source_kind: &str,
        target_path: &str,
        total_bytes: u64,
    ) -> Result<TorrentDownloadRecord, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO torrent_downloads (
              game_id, magnet_uri, save_dir, status, progress_percent,
              downloaded_bytes, total_bytes, download_speed_bytes_per_sec,
              upload_speed_bytes_per_sec, peers_count, torrent_id, error_message,
              created_at, updated_at, completed_at
            ) VALUES (?1, ?2, ?3, 'downloading', 0, 0, ?4, 0, 0, 0, NULL, NULL, ?5, ?5, NULL)
            ON CONFLICT(game_id) DO UPDATE SET
              magnet_uri = excluded.magnet_uri,
              save_dir = excluded.save_dir,
              status = excluded.status,
              progress_percent = excluded.progress_percent,
              downloaded_bytes = excluded.downloaded_bytes,
              total_bytes = excluded.total_bytes,
              download_speed_bytes_per_sec = excluded.download_speed_bytes_per_sec,
              upload_speed_bytes_per_sec = excluded.upload_speed_bytes_per_sec,
              peers_count = excluded.peers_count,
              torrent_id = excluded.torrent_id,
              error_message = excluded.error_message,
              updated_at = excluded.updated_at,
              completed_at = excluded.completed_at
            "#,
                params![
                    game_id,
                    format!("direct:{source_kind}"),
                    target_path,
                    u64_to_i64(total_bytes),
                    now
                ],
            )
            .map_err(|error| error.to_string())?;

        self.get_torrent_download(game_id)?
            .ok_or_else(|| "Direct download record was not persisted.".to_string())
```

### Завершение прямой загрузки

src-tauri/src/storage/downloads.rs, строки 637-667

```rust
    pub fn record_direct_game_download_completed(
        &self,
        game_id: &str,
        source_kind: &str,
        local_path: &str,
        sha256: &str,
        total_bytes: u64,
    ) -> Result<(DownloadRecord, TorrentDownloadRecord), String> {
        self.in_transaction(|| {
            let download =
                self.record_download(game_id, "game", Some(local_path), Some(sha256), None)?;
            self.record_direct_game_download_started(
                game_id,
                source_kind,
                local_path,
                total_bytes,
            )?;
            let torrent = self.update_torrent_progress(
                game_id,
                "completed",
                100.0,
                total_bytes,
                total_bytes,
                0,
                0,
                0,
            )?;
            Ok((download, torrent))
        })
    }

```

### Legacy download_game, обходящий реестр

src-tauri/src/commands/downloads.rs, строки 407-461

```rust
#[tauri::command]
pub async fn download_game(
    game_id: String,
    target_dir: Option<String>,
    state: State<'_, AppState>,
) -> Result<DownloadRecord, String> {
    let target_dir = resolve_target_dir_override(target_dir.as_deref())?;
    let game = {
        let store = lock_store(&state)?;
        store
            .get_game(&game_id)?
            .ok_or_else(|| format!("Unknown game: {game_id}"))?
    };

    let source = game
        .downloads
        .first()
        .ok_or_else(|| format!("Game {} has no download sources.", game.title))?;
    if matches!(
        game.content_mode.as_deref(),
        Some("user_provided" | "metadata_only")
    ) || matches!(source, SourceUri::UserProvided { .. })
    {
        return Err(format!(
            "{} is user-provided content. Import your local game file instead of starting an automatic download.",
            game.title
        ));
    }
    let destination = game_direct_download_destination(
        download_root_for_app_state(state.inner())?,
        target_dir.as_deref(),
        &game,
        source,
    );

    match download_source_to_file(source, &destination).await {
        Ok(file) => {
            let processed_path = post_process_game_archive(&game, &file.path).await?;
            let local_path = processed_path.to_string_lossy().to_string();
            let total_bytes = source_size_bytes(source).unwrap_or_else(|| file_size(&processed_path));
            let (record, _) = lock_store(&state)?.record_direct_game_download_completed(
                &game.id,
                direct_source_kind(source),
                &local_path,
                &file.sha256,
                total_bytes,
            )?;
            Ok(record)
        }
        Err(error) => {
            let _ = lock_store(&state)?.record_download(&game.id, "game", None, None, Some(&error));
            Err(error)
        }
    }
}
```

### Инвентаризация HTTP-клиентов

```text
src-tauri/src\downloads.rs:370:    let mut builder = reqwest::Client::builder()
src-tauri/src\downloads.rs:373:        .redirect(redirect_policy);
src-tauri/src\github_resolver.rs:31:    let mut builder = reqwest::Client::builder().redirect(crate::net::github_redirect_policy());
src-tauri/src\scrapers\steamgriddb.rs:37:        let client = reqwest::Client::builder()
src-tauri/src\manifest.rs:158:    let mut client_builder = reqwest::Client::builder()
src-tauri/src\scrapers\screenscraper.rs:29:        let client = reqwest::Client::builder()
src-tauri/src\scraper.rs:1194:    let client = reqwest::Client::builder()
src-tauri/src\net.rs:265:        let error = reqwest::Client::new()
src-tauri/src\net.rs:281:            resolve_and_pin_host(reqwest::Client::builder(), &loopback_url)
src-tauri/src\net.rs:288:            resolve_and_pin_host(reqwest::Client::builder(), &localhost_url)
src-tauri/src\net.rs:296:            resolve_and_pin_host(reqwest::Client::builder(), &fake_ip_url)
src-tauri/src\commands\repository_helpers.rs:36:    let mut builder = reqwest::Client::builder()
```
