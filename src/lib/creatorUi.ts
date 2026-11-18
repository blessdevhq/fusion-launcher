/** UI navigation only; descriptor-dependent editors are deliberately unavailable. */
export const creatorStages = ['base', 'composition', 'profiles', 'verification', 'release'] as const;
export type CreatorStage = (typeof creatorStages)[number];
export function creatorStageHasContent(stage: CreatorStage): boolean {
  return stage === 'base';
}
