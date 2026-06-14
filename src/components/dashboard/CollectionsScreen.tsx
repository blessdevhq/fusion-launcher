import { motion } from 'framer-motion';
import { CollectionsPanel, type CollectionTarget } from '@/components/shell/CockpitPanels';
import type { GameLibraryItem } from '@/lib/libraryStatus';

export function CollectionsScreen({
  items,
  onOpenCollection,
  onFocus
}: {
  items: GameLibraryItem[];
  onOpenCollection: (target: CollectionTarget) => void;
  onFocus: (focusId: string) => void;
}) {
  return (
    <motion.section initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} className="rh-screen rh-panel" data-testid="collections-screen">
      <CollectionsPanel items={items} onOpenCollection={onOpenCollection} onFocus={onFocus} />
    </motion.section>
  );
}
