import { motion } from 'framer-motion';
import { CollectionsPanel, HeroPanel, HomeRailsPanel, type CollectionTarget, type HomeRail } from '@/components/shell/CockpitPanels';
import type { GameLibraryItem } from '@/lib/libraryStatus';
import type { LibraryCollectionCounts, UnifiedLibraryItem } from '@/lib/unifiedLibrary';
import type { FusionCatalogEntry } from '@/types/fusionSource';
import type { CatalogGame } from '@/types/repository';
import type { BusyAction } from './types';
import { useI18n } from '@/components/I18nProvider';
import { fusionText } from '@/lib/fusionText';

interface HomeScreenProps {
  loading: boolean;
  heroItem: UnifiedLibraryItem | null;
  rails: HomeRail[];
  collectionCounts: LibraryCollectionCounts;
  busyAction: BusyAction;
  onPrimaryAction: (item: GameLibraryItem) => void;
  onOpenDetails: (game: CatalogGame) => void;
  onOpenFusion: (game: FusionCatalogEntry) => void;
  onLaunchFusion?: (game: FusionCatalogEntry) => void;
  onOpenCollection: (target: CollectionTarget) => void;
  onOpenSettings: () => void;
  onFocus: (focusId: string) => void;
  fusionCount?: number;
  onOpenFusionLibrary?: () => void;
}

export function HomeScreen({
  loading,
  heroItem,
  rails,
  collectionCounts,
  busyAction,
  onPrimaryAction,
  onOpenDetails,
  onOpenFusion,
  onLaunchFusion,
  onOpenCollection,
  onOpenSettings,
  onFocus,
  fusionCount = 0,
  onOpenFusionLibrary
}: HomeScreenProps) {
  const { locale } = useI18n();
  const text = fusionText(locale);
  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      className="rh-home-screen"
      data-testid="home-screen"
    >
      {loading ? (
        <HomeSkeleton />
      ) : (
        <>
          <HeroPanel
            heroItem={heroItem}
            busyAction={busyAction}
            onPrimaryAction={onPrimaryAction}
            onOpenDetails={onOpenDetails}
            onOpenFusion={onOpenFusion}
            onLaunchFusion={onLaunchFusion}
            onOpenSettings={onOpenSettings}
            onFocus={onFocus}
          />
          <HomeRailsPanel
            rails={rails}
            onPrimaryAction={onPrimaryAction}
            onOpenDetails={onOpenDetails}
            onOpenFusion={onOpenFusion}
            onFocus={onFocus}
          />
          {fusionCount > 0 && <button type="button" onClick={onOpenFusionLibrary} onFocus={() => onFocus('fusion-library:open')} data-focus-id="fusion-library:open" data-focus-zone="home" data-testid="home-fusion-library" className="rh-mini-action rh-focusable justify-between">{text.catalog}<span>{fusionCount} →</span></button>}
          <CollectionsPanel
            counts={collectionCounts}
            onOpenCollection={onOpenCollection}
            onFocus={onFocus}
          />
        </>
      )}
    </motion.div>
  );
}

function HomeSkeleton() {
  return (
    <div className="rh-home-skeleton" data-testid="home-skeleton" aria-hidden="true">
      <div className="sk-hero sk-block" />

      <div className="sk-toolbar">
        <div className="sk-chips">
          <span className="sk-chip sk-chip-active" />
          {Array.from({ length: 5 }).map((_, index) => (
            <span key={index} className="sk-chip sk-block" />
          ))}
        </div>
        <div className="sk-view-toggle">
          <span className="sk-view-btn sk-block" />
          <span className="sk-view-btn sk-block" />
        </div>
      </div>

      {[0, 1].map((rail) => (
        <div key={rail} className="sk-rail">
          <span className="sk-rail-label sk-block" />
          <div className="sk-rail-track">
            {Array.from({ length: 7 }).map((_, index) => (
              <div key={index} className="sk-card sk-block">
                <span className="sk-card-label" />
              </div>
            ))}
          </div>
        </div>
      ))}

      <div className="sk-bottom">
        <div className="sk-collections">
          {Array.from({ length: 4 }).map((_, index) => (
            <span key={index} className="sk-collection-card sk-block" />
          ))}
        </div>
        <div className="sk-filters-panel">
          <span className="sk-rail-label sk-block" />
          <div className="sk-filter-grid">
            {Array.from({ length: 6 }).map((_, index) => (
              <span key={index} className="sk-filter-chip sk-block" />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
