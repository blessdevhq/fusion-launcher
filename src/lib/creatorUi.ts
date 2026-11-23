/** UI navigation only; descriptor-dependent editors are deliberately unavailable. */
import type { CreatorJobStartRequest, CreatorProjectOpenResponse } from '../types/fusionCreator.ts';

type Project = CreatorProjectOpenResponse['project'];

export const creatorStages = ['base', 'composition', 'profiles', 'verification', 'release'] as const;
export type CreatorStage = (typeof creatorStages)[number];
export function creatorStageHasContent(stage: CreatorStage): boolean {
  return stage === 'base';
}

export function supportsSignedBuild(components: Project['program_descriptor']['components']): boolean {
  return components.length > 0 && components.every(({ origin, source_root }) =>
    origin.kind === 'managed' && origin.redistribution_allowed && source_root === '');
}

export function signedBuildRequest(project: Project, buildId: string, controlsId: string): CreatorJobStartRequest | null {
  const descriptor = project.program_descriptor;
  if (!descriptor.build_profiles.some(profile => profile.id === buildId && profile.controls_profile_ids.includes(controlsId))
    || !descriptor.controls_profiles.some(profile => profile.id === controlsId)) return null;
  return {
    projectId: project.project_id, projectRevision: project.revision,
    buildProfileIds: [buildId], controlsProfileIds: [controlsId],
    targetDirectory: null, releaseId: null, sourceUrl: null
  };
}
