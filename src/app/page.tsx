'use client';

import { useCallback, useEffect, useState } from 'react';
import { Dashboard } from '@/components/Dashboard';
import { OnboardingWizard } from '@/components/OnboardingWizard';
import { api } from '@/lib/api';
import type { CatalogGame, OnboardingState, RepositorySummary } from '@/types/repository';

export default function HomePage() {
  const [repositories, setRepositories] = useState<RepositorySummary[]>([]);
  const [catalog, setCatalog] = useState<CatalogGame[]>([]);
  const [onboardingState, setOnboardingState] = useState<OnboardingState | null>(null);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const [nextRepositories, nextCatalog, nextOnboardingState] = await Promise.all([
        api.listRepositories(),
        api.getCatalog(),
        api.getOnboardingState()
      ]);
      setRepositories(nextRepositories);
      setCatalog(nextCatalog);
      setOnboardingState(nextOnboardingState);
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

  const disconnectRepository = async (repositoryId: string) => {
    await api.disconnectRepository(repositoryId);
    await reload();
  };

  if (loading) {
    return (
      <main className="grid min-h-screen place-items-center bg-[#0f0f11] text-white/60">
        <div className="h-10 w-10 animate-spin rounded-full border border-white/10 border-t-hydra-accent" />
      </main>
    );
  }

  if (repositories.length === 0) {
    return (
      <OnboardingWizard
        state={onboardingState}
        catalog={catalog}
        initialMessage={message}
        onReload={reload}
      />
    );
  }

  if (onboardingState && onboardingState.step !== 'complete') {
    return (
      <OnboardingWizard
        state={onboardingState}
        catalog={catalog}
        initialMessage={message}
        onReload={reload}
      />
    );
  }

  return (
    <Dashboard
      catalog={catalog}
      repositories={repositories}
      message={message}
      onDisconnectRepository={disconnectRepository}
      onRefresh={reload}
    />
  );
}
