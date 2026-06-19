'use client';

import { useEffect, useState, type ReactNode } from 'react';
import { CheckCircle2, ChevronRight, Globe2, Link2, Loader2, Rocket } from 'lucide-react';
import { AddSourceCard } from '@/components/settings/AddSourceCard';
import { useI18n } from '@/components/I18nProvider';
import { LOCALES, type Locale } from '@/lib/i18n';
import { saveSettings, type AppSettings } from '@/lib/settings';
import type { CatalogGame, OnboardingState } from '@/types/repository';

// First-run onboarding only needs a connected source so the user can start
// adding games. Metadata and emulator setup are intentionally NOT part of the
// wizard: the emulator installs on demand when a game is installed, and both
// can be configured any time from Settings. This keeps first launch from
// forcing a download.
type WizardStep = 'welcome' | 'source' | 'ready';

interface OnboardingWizardProps {
  state: OnboardingState | null;
  catalog: CatalogGame[];
  settings: AppSettings;
  initialMessage: string | null;
  onSettingsChange: (settings: AppSettings) => void;
  onReload: () => Promise<void>;
}

const STEP_ORDER: WizardStep[] = ['welcome', 'source', 'ready'];

export function OnboardingWizard({
  state,
  catalog,
  settings,
  initialMessage,
  onSettingsChange,
  onReload
}: OnboardingWizardProps) {
  const { t } = useI18n();
  const [activeStep, setActiveStep] = useState<WizardStep>('welcome');
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(initialMessage);

  const repositoryReady = Boolean(state?.repositoriesConfigured);
  const catalogReady = catalog.length > 0;
  // A connected source is the only requirement to enter the launcher.
  const onboardingReady = repositoryReady;

  useEffect(() => {
    setMessage(initialMessage);
  }, [initialMessage]);

  // Never strand the user on the final step without a source to enter with.
  useEffect(() => {
    if (!repositoryReady) {
      setActiveStep((current) => (current === 'ready' ? 'source' : current));
    }
  }, [repositoryReady]);

  const changeLanguage = async (language: Locale) => {
    setBusy('language');
    setMessage(null);
    try {
      const nextSettings = await saveSettings({ ...settings, language });
      onSettingsChange(nextSettings);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const openLauncher = async () => {
    setBusy('open-launcher');
    setMessage(null);
    try {
      await onReload();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const nextStep = () => {
    if (activeStep === 'welcome') {
      setActiveStep(repositoryReady ? 'ready' : 'source');
      return;
    }
    if (activeStep === 'source' && repositoryReady) {
      setActiveStep('ready');
    }
  };

  return (
    <main className="rh-onboarding-screen bg-fusion-bg text-white" data-testid="onboarding-screen">
      <section className="rh-onboarding-shell" data-testid="onboarding-stepper">
        <aside className="rh-onboarding-sidebar">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src="/fusion/logo-lockup.png" alt="fusion" className="rh-onboarding-logo" />
          <div>
            <div className="rh-onboarding-eyebrow">{t.onboarding.firstRun}</div>
            <h1 className="rh-onboarding-title">{t.onboarding.title}</h1>
            <p className="rh-onboarding-copy">{t.onboarding.copy}</p>
          </div>
          <nav className="rh-onboarding-steps" aria-label={t.onboarding.stepperLabel}>
            {STEP_ORDER.map((step, index) => {
              const active = activeStep === step;
              const done = stepDone(step, repositoryReady, catalogReady, onboardingReady);
              const disabled = stepDisabled(step, repositoryReady);
              return (
                <button
                  key={step}
                  type="button"
                  onClick={() => setActiveStep(step)}
                  disabled={disabled}
                  className={`rh-onboarding-step ${active ? 'rh-onboarding-step-active' : ''}`}
                  data-testid={`onboarding-nav-${step}`}
                >
                  <span className={`rh-onboarding-step-index ${done ? 'rh-onboarding-step-done' : ''}`}>
                    {done ? <CheckCircle2 className="h-4 w-4" /> : index + 1}
                  </span>
                  <span>
                    <span className="rh-onboarding-step-title">{t.onboarding.steps[step]}</span>
                    <span className="rh-onboarding-step-detail">{stepDetail(step, repositoryReady, catalogReady, t)}</span>
                  </span>
                </button>
              );
            })}
          </nav>
        </aside>

        <section className="rh-onboarding-panel" data-testid={`onboarding-step-${activeStep}`}>
          {activeStep === 'welcome' && (
            <WelcomeStep
              language={settings.language}
              busy={busy === 'language'}
              onLanguageChange={changeLanguage}
              onNext={nextStep}
            />
          )}

          {activeStep === 'source' && (
            <SourceStep
              repositoryReady={repositoryReady}
              catalogCount={state?.catalogCount ?? catalog.length}
              onManifestInstalled={onReload}
              onNext={nextStep}
            />
          )}

          {activeStep === 'ready' && (
            <ReadyStep
              repositoryReady={repositoryReady}
              catalogReady={catalogReady}
              onboardingReady={onboardingReady}
              busy={busy}
              onOpenLauncher={openLauncher}
              onBackToSource={() => setActiveStep('source')}
            />
          )}

          {message && (
            <div className="rh-onboarding-message select-text">
              {message}
            </div>
          )}
        </section>
      </section>
    </main>
  );
}

function WelcomeStep({
  language,
  busy,
  onLanguageChange,
  onNext
}: {
  language: Locale;
  busy: boolean;
  onLanguageChange: (language: Locale) => Promise<void>;
  onNext: () => void;
}) {
  const { t } = useI18n();

  return (
    <div className="rh-onboarding-content">
      <StepHeader icon={<Rocket className="h-5 w-5" />} title={t.onboarding.welcome.title} copy={t.onboarding.welcome.copy} />
      <div className="rh-onboarding-card">
        <div className="flex items-start gap-3">
          <Globe2 className="mt-1 h-5 w-5 text-fusion-accent" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-black text-white/90">{t.language.label}</div>
            <p className="mt-1 text-sm leading-6 text-white/52">{t.language.description}</p>
            <select
              value={language}
              onChange={(event) => void onLanguageChange(event.target.value as Locale)}
              disabled={busy}
              className="mt-4 h-11 w-full rounded-sm border border-white/10 bg-black/40 px-3 text-sm font-semibold text-white/80 outline-none transition focus:border-white/60"
              data-testid="onboarding-language"
            >
              {LOCALES.map((item) => (
                <option key={item} value={item}>{t.language.options[item]}</option>
              ))}
            </select>
          </div>
        </div>
      </div>
      <div className="rh-onboarding-actions">
        <button type="button" onClick={onNext} className="rh-primary-action" data-testid="onboarding-next">
          {t.onboarding.continue}
          <ChevronRight className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}

function SourceStep({
  repositoryReady,
  catalogCount,
  onManifestInstalled,
  onNext
}: {
  repositoryReady: boolean;
  catalogCount: number;
  onManifestInstalled: () => Promise<void>;
  onNext: () => void;
}) {
  const { t } = useI18n();

  return (
    <div className="rh-onboarding-content">
      <StepHeader icon={<Link2 className="h-5 w-5" />} title={t.onboarding.sourceStep.title} copy={t.onboarding.sourceStep.copy} />
      <AddSourceCard onAdded={onManifestInstalled} />
      <div className="rh-onboarding-checkline">
        <SetupItem done={repositoryReady} title={t.onboarding.setup.source} detail={repositoryReady ? t.onboarding.sourceStep.connected(catalogCount) : t.onboarding.sourceStep.notConnected} />
      </div>
      <div className="rh-onboarding-actions">
        <button type="button" onClick={onNext} disabled={!repositoryReady} className="rh-primary-action" data-testid="onboarding-next-source">
          {t.onboarding.continue}
          <ChevronRight className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}

function ReadyStep({
  repositoryReady,
  catalogReady,
  onboardingReady,
  busy,
  onOpenLauncher,
  onBackToSource
}: {
  repositoryReady: boolean;
  catalogReady: boolean;
  onboardingReady: boolean;
  busy: string | null;
  onOpenLauncher: () => Promise<void>;
  onBackToSource: () => void;
}) {
  const { t } = useI18n();

  return (
    <div className="rh-onboarding-content">
      <StepHeader icon={<CheckCircle2 className="h-5 w-5" />} title={t.onboarding.readyStep.title} copy={t.onboarding.readyStep.copy} />
      <div className="rh-onboarding-checkline">
        <SetupItem done={repositoryReady} title={t.onboarding.setup.source} detail={repositoryReady ? t.onboarding.readyStep.sourceReady : t.onboarding.sourceStep.notConnected} />
        <SetupItem done={catalogReady} title={t.onboarding.readyStep.catalog} detail={catalogReady ? t.onboarding.readyStep.catalogReady : t.onboarding.readyStep.catalogMissing} />
      </div>

      <div className="rh-onboarding-actions">
        {!repositoryReady && (
          <button type="button" onClick={onBackToSource} className="rh-mini-action">{t.onboarding.readyStep.fixSource}</button>
        )}
        <button type="button" onClick={onOpenLauncher} disabled={!onboardingReady || busy !== null} className="rh-primary-action" data-testid="onboarding-open-launcher">
          {busy === 'open-launcher' ? <Loader2 className="h-4 w-4 animate-spin" /> : <Rocket className="h-4 w-4" />}
          {t.onboarding.readyStep.openLauncher}
        </button>
      </div>
    </div>
  );
}

function StepHeader({ icon, title, copy }: { icon: ReactNode; title: string; copy: string }) {
  return (
    <header className="rh-onboarding-step-header">
      <div className="rh-onboarding-step-icon">{icon}</div>
      <div>
        <h2>{title}</h2>
        <p>{copy}</p>
      </div>
    </header>
  );
}

function SetupItem({ done, title, detail }: { done: boolean; title: string; detail: string }) {
  return (
    <div className={`rounded-sm border p-4 ${done ? 'border-fusion-accent/24 bg-fusion-accent/10' : 'border-white/10 bg-black/18'}`}>
      <div className="flex items-center gap-2">
        <CheckCircle2 className={`h-4 w-4 ${done ? 'text-emerald-200' : 'text-white/22'}`} />
        <span className="font-black">{title}</span>
      </div>
      <div className="mt-2 truncate text-xs text-white/46">{detail}</div>
    </div>
  );
}

function stepDone(
  step: WizardStep,
  repositoryReady: boolean,
  catalogReady: boolean,
  onboardingReady: boolean
) {
  if (step === 'welcome') return true;
  if (step === 'source') return repositoryReady && catalogReady;
  return onboardingReady;
}

function stepDisabled(step: WizardStep, repositoryReady: boolean) {
  if (step === 'ready') return !repositoryReady;
  return false;
}

function stepDetail(
  step: WizardStep,
  repositoryReady: boolean,
  catalogReady: boolean,
  t: ReturnType<typeof useI18n>['t']
) {
  if (step === 'welcome') return t.onboarding.stepDetails.welcome;
  if (step === 'source') return repositoryReady && catalogReady ? t.common.ready : t.onboarding.stepDetails.source;
  return repositoryReady ? t.common.ready : t.onboarding.stepDetails.ready;
}
