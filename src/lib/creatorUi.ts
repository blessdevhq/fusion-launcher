/** UI navigation and pure profile/release/report predicates over Creator contracts. */
import type {
  CreatorJobProgress,
  CreatorJobStartRequest,
  CreatorProjectOpenResponse,
  CreatorRelease,
  CreatorReport,
  CreatorReportKind,
  CreatorReportStatus
} from '../types/fusionCreator.ts';

type Project = CreatorProjectOpenResponse['project'];
type Descriptor = Project['program_descriptor'];

export const creatorStages = ['base', 'composition', 'profiles', 'verification', 'release'] as const;
export type CreatorStage = (typeof creatorStages)[number];
export function creatorStageHasContent(stage: CreatorStage): boolean {
  return stage === 'base' || stage === 'verification' || stage === 'release';
}

export const creatorReportKinds: readonly CreatorReportKind[] = [
  'structure_integrity',
  'clean_install',
  'observed_launch',
  'manual_scenario',
  'https_acquisition'
];

export function supportsSignedBuild(components: Descriptor['components']): boolean {
  return components.length > 0 && components.every(({ origin, source_root }) =>
    origin.kind === 'managed' && origin.redistribution_allowed && source_root === '');
}

export function supportsCleanVerify(descriptor: Descriptor): boolean {
  return supportsSignedBuild(descriptor.components) && descriptor.entrypoint.environment_paths.length === 0;
}

function profileRequest(
  project: Project,
  buildId: string,
  controlsId: string,
  releaseId: string | null,
  targetDirectory: string | null = null,
  sourceUrl: string | null = null
): CreatorJobStartRequest | null {
  const descriptor = project.program_descriptor;
  if (!descriptor.build_profiles.some(profile => profile.id === buildId && profile.controls_profile_ids.includes(controlsId))
    || !descriptor.controls_profiles.some(profile => profile.id === controlsId)) return null;
  return {
    projectId: project.project_id, projectRevision: project.revision,
    buildProfileIds: [buildId], controlsProfileIds: [controlsId],
    releaseId, targetDirectory, sourceUrl
  };
}

export function signedBuildRequest(project: Project, buildId: string, controlsId: string): CreatorJobStartRequest | null {
  return profileRequest(project, buildId, controlsId, null);
}

export function verifyReleaseRequest(
  project: Project,
  releaseId: string,
  buildId: string,
  controlsId: string
): CreatorJobStartRequest | null {
  if (!releaseId) return null;
  return profileRequest(project, buildId, controlsId, releaseId);
}

export function currentRevisionReleases(
  releases: readonly CreatorRelease[],
  project: Pick<Project, 'project_id' | 'revision'>
): CreatorRelease[] {
  return releases.filter(release =>
    release.projectId === project.project_id && release.projectRevision === project.revision);
}

export interface DeclaredProfilePair {
  buildId: string;
  buildTitle: string;
  controlsId: string;
  controlsTitle: string;
}

export function declaredProfilePairs(descriptor: Descriptor): DeclaredProfilePair[] {
  const controlsById = new Map(descriptor.controls_profiles.map(profile => [profile.id, profile.title]));
  const pairs: DeclaredProfilePair[] = [];
  const seen = new Set<string>();
  for (const build of descriptor.build_profiles) {
    for (const controlsId of build.controls_profile_ids) {
      const controlsTitle = controlsById.get(controlsId);
      const key = `${build.id}\u0000${controlsId}`;
      if (controlsTitle === undefined || seen.has(key)) continue;
      seen.add(key);
      pairs.push({ buildId: build.id, buildTitle: build.title, controlsId, controlsTitle });
    }
  }
  return pairs;
}

export function canResumeInterruptedJob(entry: {
  acceptingEvents: boolean;
  progress: Pick<CreatorJobProgress, 'status'> | null;
  job: { checkpoints: readonly unknown[] } | null;
}): boolean {
  return entry.acceptingEvents
    && entry.progress?.status === 'interrupted'
    && (entry.job?.checkpoints.length ?? 0) > 0;
}

export interface PairReportSummary {
  status: CreatorReportStatus;
  report: CreatorReport | null;
}

export function summarizePairReports(
  release: CreatorRelease,
  reports: Record<string, { report: CreatorReport | null } | CreatorReport | null | undefined>,
  buildId: string,
  controlsId: string
): Record<CreatorReportKind, PairReportSummary> {
  const summary = Object.fromEntries(
    creatorReportKinds.map(kind => [kind, { status: 'not_tested' as CreatorReportStatus, report: null }])
  ) as Record<CreatorReportKind, PairReportSummary>;
  for (const reportId of release.reportIds) {
    const raw = Object.hasOwn(reports, reportId) ? reports[reportId] : undefined;
    const report = raw && typeof raw === 'object' && 'report' in raw ? raw.report : raw;
    if (!report
      || report.reportId !== reportId
      || report.releaseId !== release.releaseId
      || report.releaseBlake3 !== release.manifestBlake3
      || report.projectRevision !== release.projectRevision
      || report.buildProfileId !== buildId
      || report.controlsProfileId !== controlsId) continue;
    const current = summary[report.kind];
    if (report.status === 'failed' || (report.status === 'passed' && current.status !== 'failed')) {
      summary[report.kind] = { status: report.status, report };
    }
  }
  return summary;
}

export interface ReleaseExportReadiness {
  ready: boolean;
  hasFailedReport: boolean;
  missingPairs: DeclaredProfilePair[];
}

export function isReleaseExportReady(
  project: Project,
  release: CreatorRelease | null,
  reports: Record<string, { report: CreatorReport | null } | CreatorReport | null | undefined>
): ReleaseExportReadiness {
  const pairs = declaredProfilePairs(project.program_descriptor);
  if (!release || release.projectId !== project.project_id || release.projectRevision !== project.revision || pairs.length === 0) {
    return { ready: false, hasFailedReport: false, missingPairs: pairs };
  }
  let hasFailedReport = false;
  const missingPairs: DeclaredProfilePair[] = [];
  for (const pair of pairs) {
    const summary = summarizePairReports(release, reports, pair.buildId, pair.controlsId);
    if (summary.structure_integrity.status === 'failed' || summary.clean_install.status === 'failed') {
      hasFailedReport = true;
    }
    if (summary.structure_integrity.status !== 'passed' || summary.clean_install.status !== 'passed') {
      missingPairs.push(pair);
    }
  }
  return {
    ready: !hasFailedReport && missingPairs.length === 0,
    hasFailedReport,
    missingPairs
  };
}

export function exportReleaseRequest(
  project: Project,
  releaseId: string,
  targetDirectory: string
): CreatorJobStartRequest | null {
  const trimmedDir = targetDirectory.trim();
  const [firstPair] = declaredProfilePairs(project.program_descriptor);
  if (!releaseId || !trimmedDir || !firstPair) return null;
  return profileRequest(project, firstPair.buildId, firstPair.controlsId, releaseId, trimmedDir, null);
}

export function verifyHttpsRequest(
  project: Project,
  releaseId: string,
  sourceUrl: string,
  buildId: string,
  controlsId: string
): CreatorJobStartRequest | null {
  const trimmedUrl = sourceUrl.trim();
  if (!releaseId || !trimmedUrl.startsWith('https://') || trimmedUrl.endsWith('/')) return null;
  return profileRequest(project, buildId, controlsId, releaseId, null, trimmedUrl);
}
