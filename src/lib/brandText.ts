const LEGACY_PRODUCT_NAME = ['Retro', 'Hydra'].join('');
const LEGACY_PRODUCT_PATTERN = new RegExp(`\\b${LEGACY_PRODUCT_NAME}\\b`, 'g');

export function displayProductText(value: string) {
  return value.replace(LEGACY_PRODUCT_PATTERN, 'Fusion Launcher');
}
