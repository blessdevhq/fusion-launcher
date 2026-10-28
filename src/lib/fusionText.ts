import type { Locale } from './i18n.ts';

const en = {
  localFolder: 'Choose source folder', localSource: 'Local source', replaceFolder: 'Choose another release folder',
  localSourceNote: 'Select a folder containing source-index.json and its artifacts. Adding it does not install games.',
  prepareEden: 'Prepare portable Eden profiles', prepareEdenNote: 'For Eden sources: relocate the isolated data paths and register the game/update directory before launch.',
  catalog: 'Fusion Source games', details: 'Open details', ready: 'Ready to launch', installed: 'Installed', available: 'Available',
  install: 'Install', update: 'Update', rebuild: 'Rebuild', repair: 'Restore files', updateAvailable: 'Update available', launch: 'Launch', rollback: 'Roll back binaries', remove: 'Remove installation', cancel: 'Cancel operation',
  addons: 'Optional layers', runtime: 'Runtime channel', pinned: 'Pinned', dynamic: 'Dynamic', apply: 'Apply options',
  blockers: 'Needs attention', warnings: 'Source notes', files: 'Your game and system files', choose: 'Choose file', chooseFolder: 'Choose folder',
  rawPath: 'Relative destination for this raw file', trust: 'Approve launch', trusted: 'Hash approved',
  crashes: 'Crash reports', previewCrash: 'View report', send: 'Send this report', discard: 'Delete', exitCode: 'Exit code', crashSignature: 'Signature', crashReport: 'Report',
  noEndpoint: 'This source has no report endpoint.', endpoint: 'Send to', close: 'Close', loading: 'Loading game manifest…',
  source: 'Source', version: 'Version', noPlatform: 'Native / custom runtime', rollbackNote: 'Binary rollback keeps your current saves.',
  removeConfirm: 'Remove this Fusion installation? Your saves will be preserved.', removeSourceConfirm: 'Disconnect this Fusion source? Remove its installed games first. Your saves will be preserved.',
  externalRoot: 'Workspace directory on the base game volume (optional)', workspaceManaged: 'Managed library directory', workspaceLocked: 'To change the workspace directory, remove this installation and install it again. Your saves will be preserved.', operation: 'Operation', more: 'Show more games',
  previewNote: 'Browser preview uses simulated files and processes.', publicKey: 'Public key', noKey: 'Not provided',
  signatureNote: 'Signed indexes are verified against the source key. Executable approval is separate.', indexNote: 'Adding a source loads only its index. Installation starts when you choose Install.',
  refresh: 'Refresh', disconnect: 'Disconnect', noMatches: 'No Fusion games match this search.', optionsSaved: 'Settings applied.', optionsApplied: 'Settings applied', optionsOnInstall: 'Settings will be applied during installation.', noAddons: 'No optional layers.', noRuntime: 'This game uses its own runtime.', perSecond: '/s',
  integrityAttention: 'Files need your attention', integrityRepair: 'A required file is missing or changed. Restore the files before launching.', integrityTrust: 'Review the files below and approve each exact hash before launching.', localBaseModified: 'The original game files changed. Rebuild the workspace from your current installation.',
  verifiedFiles: (count: number) => `Verified and approved: ${count} executable ${count === 1 ? 'file' : 'files'}`,
  operationPhase: (phase: string) => ({ install: 'Preparing installation', update: 'Preparing rebuild', rollback: 'Rolling back binaries', launch: 'Starting game', remove: 'Removing installation', manifest: 'Checking source', resolveRuntime: 'Resolving runtime', fetch: 'Downloading files', transform: 'Preparing layer', assemble: 'Building workspace', integrity: 'Checking files', running: 'Game is running', exec: 'Game is running', cancelling: 'Cancelling operation', failed: 'Operation failed' }[phase] ?? 'Preparing operation'),
  reported: 'Report sent.', discarded: 'Report discarded.', error: 'Operation failed', progress: 'Fusion operations',
};

const ru: typeof en = {
  localFolder: 'Выбрать папку Source', localSource: 'Локальный источник', replaceFolder: 'Выбрать папку нового выпуска',
  localSourceNote: 'Выберите папку с source-index.json и артефактами. Подключение не устанавливает игры.',
  prepareEden: 'Подготавливать portable-профили Eden', prepareEdenNote: 'Для источников Eden: перед запуском обновлять пути изолированного профиля и каталог игры/обновления.',
  catalog: 'Игры Fusion Source', details: 'Открыть подробности', ready: 'Готово к запуску', installed: 'Установлено', available: 'Доступно',
  install: 'Установить', update: 'Обновить', rebuild: 'Пересобрать', repair: 'Восстановить файлы', updateAvailable: 'Доступно обновление', launch: 'Запустить', rollback: 'Откатить бинарники', remove: 'Удалить установку', cancel: 'Отменить операцию',
  addons: 'Дополнительные слои', runtime: 'Канал рантайма', pinned: 'Закреплённый', dynamic: 'Динамический', apply: 'Применить настройки',
  blockers: 'Требуется внимание', warnings: 'Примечания источника', files: 'Ваши файлы игры и системы', choose: 'Выбрать файл', chooseFolder: 'Выбрать папку',
  rawPath: 'Относительный путь назначения файла', trust: 'Одобрить запуск', trusted: 'Хеш одобрен',
  crashes: 'Краш-отчёты', previewCrash: 'Просмотреть отчёт', send: 'Отправить этот отчёт', discard: 'Удалить', exitCode: 'Код завершения', crashSignature: 'Сигнатура', crashReport: 'Отчёт',
  noEndpoint: 'Источник не указал адрес для отчётов.', endpoint: 'Адрес отправки', close: 'Закрыть', loading: 'Загрузка манифеста игры…',
  source: 'Источник', version: 'Версия', noPlatform: 'Native / собственный runtime', rollbackNote: 'Откат бинарников сохраняет текущие сейвы.',
  removeConfirm: 'Удалить эту установку Fusion? Сохранения останутся.', removeSourceConfirm: 'Отключить источник Fusion? Сначала удалите его установленные игры. Сохранения останутся.',
  externalRoot: 'Каталог workspace на томе базовой игры (необязательно)', workspaceManaged: 'Управляемый каталог библиотеки', workspaceLocked: 'Чтобы изменить каталог workspace, удалите установку и установите игру заново. Сохранения останутся.', operation: 'Операция', more: 'Показать ещё игры',
  previewNote: 'В браузерном предпросмотре файлы и процессы моделируются.', publicKey: 'Публичный ключ', noKey: 'Не указан',
  signatureNote: 'Подписанный индекс проверяется по ключу источника. Доверие исполняемым файлам подтверждается отдельно.', indexNote: 'Добавление источника загружает только индекс. Установка начнётся по кнопке «Установить».',
  refresh: 'Обновить', disconnect: 'Отключить', noMatches: 'Нет игр Fusion по этому запросу.', optionsSaved: 'Настройки применены.', optionsApplied: 'Настройки применены', optionsOnInstall: 'Настройки будут применены при установке.', noAddons: 'Дополнительных слоёв нет.', noRuntime: 'Игра использует собственный runtime.', perSecond: '/с',
  integrityAttention: 'Нужно проверить файлы', integrityRepair: 'Нужный файл отсутствует или изменён. Восстановите файлы перед запуском.', integrityTrust: 'Проверьте файлы ниже и подтвердите доверие каждому точному хешу перед запуском.', localBaseModified: 'Исходные файлы игры изменились. Пересоберите workspace из текущей установки.',
  verifiedFiles: (count: number) => { const last = count % 10; const teens = count % 100 >= 11 && count % 100 <= 14; return `Проверено и одобрено: ${count} ${!teens && last === 1 ? 'исполняемый файл' : !teens && last >= 2 && last <= 4 ? 'исполняемых файла' : 'исполняемых файлов'}`; },
  operationPhase: (phase: string) => ({ install: 'Подготовка установки', update: 'Подготовка пересборки', rollback: 'Откат бинарников', launch: 'Запуск игры', remove: 'Удаление установки', manifest: 'Проверка источника', resolveRuntime: 'Выбор версии runtime', fetch: 'Загрузка файлов', transform: 'Подготовка слоя', assemble: 'Сборка workspace', integrity: 'Проверка файлов', running: 'Игра запущена', exec: 'Игра запущена', cancelling: 'Отмена операции', failed: 'Операция завершилась ошибкой' }[phase] ?? 'Подготовка операции'),
  reported: 'Отчёт отправлен.', discarded: 'Отчёт удалён.', error: 'Операция завершилась ошибкой', progress: 'Операции Fusion',
};

export function fusionText(locale: Locale) { return locale === 'ru' ? ru : en; }
