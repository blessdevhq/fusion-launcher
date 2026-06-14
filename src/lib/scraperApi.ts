import { api } from './api.ts';
import type {
  LibraryScrapeStatus,
  ManualGameMetadataInput,
  ScrapeCandidate,
  ScrapeState,
  ScreenScraperStatus,
  SteamGridDbStatus
} from '@/types/repository';

export type { LibraryScrapeStatus, ScrapeCandidate, ScrapeState, ScreenScraperStatus, SteamGridDbStatus };

export const scraperApi = {
  scrapeGame(gameId: string) {
    return api.scrapeGame(gameId);
  },
  getScrapeState(gameId: string) {
    return api.getScrapeState(gameId);
  },
  listScrapeCandidates(gameId: string) {
    return api.listScrapeCandidates(gameId);
  },
  applyScrapeOverride(gameId: string, providerGameId: string) {
    return api.applyScrapeOverride(gameId, providerGameId);
  },
  saveManualMetadata(gameId: string, metadata: ManualGameMetadataInput) {
    return api.saveManualMetadata(gameId, metadata);
  },
  clearScrapeOverride(gameId: string) {
    return api.clearScrapeOverride(gameId);
  },
  saveScreenscraperCredentials(ssid: string, sspassword: string, region?: string) {
    return api.saveScreenscraperCredentials(ssid, sspassword, region);
  },
  getScreenscraperStatus(): Promise<ScreenScraperStatus> {
    return api.getScreenscraperStatus();
  },
  saveSteamgriddbKey(apiKey: string): Promise<SteamGridDbStatus> {
    return api.saveSteamgriddbKey(apiKey);
  },
  getSteamgriddbStatus(): Promise<SteamGridDbStatus> {
    return api.getSteamgriddbStatus();
  },
  scrapeLibrary(): Promise<LibraryScrapeStatus> {
    return api.scrapeLibrary();
  },
  cancelLibraryScrape(): Promise<LibraryScrapeStatus> {
    return api.cancelLibraryScrape();
  }
};
