import type { Locale } from './i18n.ts';

const en = {
  catalog: 'Fusion Source games', details: 'Open details', ready: 'Ready to launch', installed: 'Installed', available: 'Available',
  install: 'Install', update: 'Update', launch: 'Launch', rollback: 'Roll back binaries', remove: 'Remove installation', cancel: 'Cancel operation',
  addons: 'Optional layers', runtime: 'Runtime channel', pinned: 'Pinned', dynamic: 'Dynamic', apply: 'Apply options',
  blockers: 'Needs attention', warnings: 'Source notes', files: 'Your game and system files', choose: 'Choose file', chooseFolder: 'Choose folder',
  rawPath: 'Relative destination for this raw file', trust: 'Trust this executable hash', trusted: 'Hash approved',
  crashes: 'Crash reports', previewCrash: 'Review cleaned report', send: 'Send this report', discard: 'Discard report',
  noEndpoint: 'This source has no report endpoint.', endpoint: 'Send to', close: 'Close', loading: 'Loading game manifest…',
  source: 'Source', version: 'Version', noPlatform: 'Native / custom runtime', rollbackNote: 'Binary rollback keeps your current saves.',
  removeConfirm: 'Remove this Fusion installation? Your saves will be preserved.', removeSourceConfirm: 'Disconnect this Fusion source? Remove its installed games first. Your saves will be preserved.',
  externalRoot: 'Workspace directory on the base game volume (optional)', operation: 'Operation', more: 'Show more games',
  previewNote: 'Browser preview uses simulated files and processes.', publicKey: 'Public key', noKey: 'Not provided',
  signatureNote: 'Public keys are metadata; v1 does not verify signatures.', indexNote: 'Adding a source loads only its index. Installation starts when you choose Install.',
  refresh: 'Refresh', disconnect: 'Disconnect', noMatches: 'No Fusion games match this search.', optionsSaved: 'Options saved. Update the installation to apply binary changes.',
  reported: 'Report sent.', discarded: 'Report discarded.', error: 'Operation failed', progress: 'Fusion operations',
};

const ru: typeof en = {
  catalog: 'Игры Fusion Source', details: 'Открыть подробности', ready: 'Готово к запуску', installed: 'Установлено', available: 'Доступно',
  install: 'Установить', update: 'Обновить', launch: 'Запустить', rollback: 'Откатить бинарники', remove: 'Удалить установку', cancel: 'Отменить операцию',
  addons: 'Дополнительные слои', runtime: 'Канал рантайма', pinned: 'Закреплённый', dynamic: 'Динамический', apply: 'Применить настройки',
  blockers: 'Требуется внимание', warnings: 'Примечания источника', files: 'Ваши файлы игры и системы', choose: 'Выбрать файл', chooseFolder: 'Выбрать папку',
  rawPath: 'Относительный путь назначения файла', trust: 'Доверять этому хешу executable', trusted: 'Хеш одобрен',
  crashes: 'Краш-отчёты', previewCrash: 'Просмотреть очищенный отчёт', send: 'Отправить этот отчёт', discard: 'Удалить отчёт',
  noEndpoint: 'Источник не указал адрес для отчётов.', endpoint: 'Адрес отправки', close: 'Закрыть', loading: 'Загрузка манифеста игры…',
  source: 'Источник', version: 'Версия', noPlatform: 'Native / собственный runtime', rollbackNote: 'Откат бинарников сохраняет текущие сейвы.',
  removeConfirm: 'Удалить эту установку Fusion? Сохранения останутся.', removeSourceConfirm: 'Отключить источник Fusion? Сначала удалите его установленные игры. Сохранения останутся.',
  externalRoot: 'Каталог workspace на томе базовой игры (необязательно)', operation: 'Операция', more: 'Показать ещё игры',
  previewNote: 'В браузерном предпросмотре файлы и процессы моделируются.', publicKey: 'Публичный ключ', noKey: 'Не указан',
  signatureNote: 'Публичный ключ — метаданные; v1 не проверяет подписи.', indexNote: 'Добавление источника загружает только индекс. Установка начнётся по кнопке «Установить».',
  refresh: 'Обновить', disconnect: 'Отключить', noMatches: 'Нет игр Fusion по этому запросу.', optionsSaved: 'Настройки сохранены. Обновите установку, чтобы применить изменения бинарников.',
  reported: 'Отчёт отправлен.', discarded: 'Отчёт удалён.', error: 'Операция завершилась ошибкой', progress: 'Операции Fusion',
};

export function fusionText(locale: Locale) { return locale === 'ru' ? ru : en; }
