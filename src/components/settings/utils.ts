import type { Locale, UiText } from '@/lib/i18n';
import type { EmulatorDraftTone } from '@/lib/settingsModalState';
import type { SettingsSection, UpdatePanelPhase } from './types';

export function sectionTitle(section: SettingsSection, t: UiText) {
  return t.settings.sections[section];
}

export function updatePhaseLabel(phase: UpdatePanelPhase, t: UiText) {
  return t.settings.updatePhase[phase];
}

export function statusToneClass(tone: EmulatorDraftTone) {
  if (tone === 'valid') return 'border-emerald-200/[0.24] bg-emerald-200/10 text-emerald-100';
  if (tone === 'missing') return 'border-amber-200/[0.24] bg-amber-200/10 text-amber-100';
  if (tone === 'invalid') return 'border-red-200/[0.24] bg-red-200/10 text-red-100';
  if (tone === 'unsaved') return 'border-white/[0.24] bg-white/[0.09] text-white/[0.82]';
  return 'border-white/[0.12] bg-white/[0.04] text-white/[0.42]';
}

export function healthStatusLabel(status: string, t: UiText) {
  if (status === 'ready') return t.settings.healthStatus.ready;
  if (status === 'missing') return t.settings.healthStatus.missing;
  if (status === 'error') return t.settings.healthStatus.error;
  if (status === 'corrupt') return t.settings.healthStatus.corrupt;
  return status;
}

export function healthToneClass(status: string) {
  if (status === 'ready') return 'bg-fusion-green';
  if (status === 'corrupt' || status === 'error') return 'bg-red-300';
  return 'bg-amber-300';
}

export function formatDateTime(timestamp: string, locale: Locale) {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return timestamp;
  return date.toLocaleString(locale, { dateStyle: 'short', timeStyle: 'short' });
}

export function shortHash(value: string) {
  if (value.length <= 16) return value;
  return `${value.slice(0, 8)}...${value.slice(-8)}`;
}

export function trustBadgeClass(trustLevel: string) {
  if (trustLevel === 'official') return 'border-emerald-300/[0.24] bg-emerald-300/10 text-emerald-100';
  if (trustLevel === 'community') return 'border-fusion-accent/[0.24] bg-fusion-accent/10 text-fusion-accent';
  return 'border-amber-300/[0.24] bg-amber-300/10 text-amber-100';
}

export function getFocusableElements(container: HTMLElement | null): HTMLElement[] {
  if (!container) return [];
  return Array.from(
    container.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [href], [tabindex]:not([tabindex="-1"])'
    )
  ).filter((element) => !element.hasAttribute('disabled') && element.tabIndex !== -1);
}
