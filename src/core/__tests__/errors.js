export class FusionLauncherError extends Error {
  constructor(code, message, options = {}) {
    super(message);
    this.name = 'FusionLauncherError';
    this.code = code;
    this.friendlyMessage = options.friendlyMessage || message;
    this.meta = options.meta || {};
    this.cause = options.cause;
  }
}

export function toFriendlyError(error) {
  if (error instanceof FusionLauncherError) {
    return {
      code: error.code,
      message: error.friendlyMessage,
      technicalMessage: error.message,
      meta: error.meta
    };
  }

  const rawMessage = error?.message || String(error || 'Unknown error');
  const message = rawMessage.toLowerCase();

  if (message.includes('premium subscription')) {
    return {
      code: 'premium_lock',
      message: 'Эта платформа доступна в Premium. Включите подписку в настройках, чтобы продолжить.',
      technicalMessage: rawMessage,
      meta: {}
    };
  }

  if (message.includes('system files manifest') || message.includes('manifest url is not configured')) {
    return {
      code: 'missing_system_files',
      message: 'Для этой платформы нужны пользовательские BIOS, keys или firmware. Манифест системных файлов пока не настроен.',
      message: 'Для этой платформы нужны BIOS или ключи. Добавьте Community Source (Источник) в настройках, чтобы лончер мог их скачать.',
      technicalMessage: rawMessage,
      meta: {}
    };
  }

  if (message.includes('magnet link is not configured') || message.includes('download url is not configured')) {
    return {
      code: 'placeholder_source',
      message: 'Источник загрузки пока не настроен. Импортируйте локальный файл или включите demo mode.',
      message: 'Игра не найдена в ваших источниках. Добавьте URL каталога сообщества в настройках или импортируйте локальный файл.',
      technicalMessage: rawMessage,
      meta: {}
    };
  }

  if (message.includes('sha256 mismatch') || message.includes('md5 mismatch')) {
    return {
      code: 'hash_mismatch',
      message: 'Контрольная сумма файла не совпала. Проверьте источник файла и попробуйте импортировать заново.',
      technicalMessage: rawMessage,
      meta: {}
    };
  }

  if (message.includes('enoent') || message.includes('executable not found') || message.includes('spawn')) {
    return {
      code: 'missing_sidecar',
      message: 'Не найден нужный исполняемый файл. Проверьте диагностику и установку sidecar/emulator binaries.',
      technicalMessage: rawMessage,
      meta: {}
    };
  }

  if (message.includes('unsupported file extension') || message.includes('unsupported platform')) {
    return {
      code: 'unsupported_import',
      message: 'Этот файл не похож на поддерживаемый ROM/образ. Выберите файл NES, SNES, Genesis, PS1, PS2, Switch или PS3.',
      technicalMessage: rawMessage,
      meta: {}
    };
  }

  if (message.includes('no local games')) {
    return {
      code: 'no_local_games',
      message: 'Локальные игры пока не импортированы. Нажмите «Импорт» и выберите свой ROM или образ.',
      technicalMessage: rawMessage,
      meta: {}
    };
  }

  return {
    code: 'unknown',
    message: rawMessage,
    technicalMessage: rawMessage,
    meta: {}
  };
}

export function serializeError(error) {
  const friendly = toFriendlyError(error);
  return {
    ...friendly,
    timestamp: new Date().toISOString()
  };
}
