'use client';

import { useCallback, useEffect, useState } from 'react';
import { Dashboard } from '@/components/Dashboard';
import { I18nProvider } from '@/components/I18nProvider';
import { OnboardingWizard } from '@/components/OnboardingWizard';
import { api } from '@/lib/api';
import { DEFAULT_SETTINGS, loadSettings, type AppSettings } from '@/lib/settings';
import type { CatalogGame, OnboardingState, RepositorySummary } from '@/types/repository';

export default function HomePage() {
  const [repositories, setRepositories] = useState<RepositorySummary[]>([]);
  const [catalog, setCatalog] = useState<CatalogGame[]>([]);
  const [onboardingState, setOnboardingState] = useState<OnboardingState | null>(null);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      await api.repairLibrary();
      const [nextRepositories, nextCatalog, nextOnboardingState, nextSettings] = await Promise.all([
        api.listRepositories(),
        api.getCatalog(),
        api.getOnboardingState(),
        loadSettings().catch(() => DEFAULT_SETTINGS)
      ]);
      setRepositories(nextRepositories);
      setCatalog(nextCatalog);
      setOnboardingState(nextOnboardingState);
      setSettings(nextSettings);
      setMessage(null);
    } catch (error) {
      setRepositories([]);
      setCatalog([]);
      setOnboardingState(null);
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    document.documentElement.lang = settings.language;
  }, [settings.language]);

  const disconnectRepository = async (repositoryId: string) => {
    await api.disconnectRepository(repositoryId);
    await reload();
  };
  const metadataOnboardingReady = settings.metadataOnboarding.complete;

  if (loading) {
    return (
      <main className="grid min-h-screen place-items-center bg-fusion-bg text-white/60">
        <div className="rh-boot-loader">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src="/fusion/mascot-head.png" alt="" className="rh-boot-mascot" />
          <div className="rh-boot-spinner" />
        </div>
      </main>
    );
  }

  if (repositories.length === 0) {
    return (
      <I18nProvider locale={settings.language}>
        <OnboardingWizard
          state={onboardingState}
          catalog={catalog}
          settings={settings}
          initialMessage={message}
          onSettingsChange={setSettings}
          onReload={reload}
        />
      </I18nProvider>
    );
  }

  if ((onboardingState && onboardingState.step !== 'complete') || !metadataOnboardingReady) {
    return (
      <I18nProvider locale={settings.language}>
        <OnboardingWizard
          state={onboardingState}
          catalog={catalog}
          settings={settings}
          initialMessage={message}
          onSettingsChange={setSettings}
          onReload={reload}
        />
      </I18nProvider>
    );
  }

  return (
    <Dashboard
      initialSettings={settings}
      catalog={catalog}
      repositories={repositories}
      message={message}
      onDisconnectRepository={disconnectRepository}
      onRefresh={reload}
    />
  );
}
