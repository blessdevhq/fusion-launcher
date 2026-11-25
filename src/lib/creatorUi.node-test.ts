import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import {
  canResumeInterruptedJob,
  creatorStages,
  creatorStageHasContent,
  currentRevisionReleases,
  declaredProfilePairs,
  exportReleaseRequest,
  isReleaseExportReady,
  signedBuildRequest,
  summarizePairReports,
  supportsCleanVerify,
  supportsSignedBuild,
  verifyHttpsRequest,
  verifyReleaseRequest
} from './creatorUi.ts';
import type { CreatorProjectOpenResponse, CreatorRelease, CreatorReport } from '../types/fusionCreator.ts';

const contracts = JSON.parse(readFileSync(new URL('../../fixtures/creator/contracts.json', import.meta.url), 'utf8')) as {
  project: CreatorProjectOpenResponse['project'];
  report: CreatorReport;
  resumeStamp: CreatorProjectOpenResponse['project']['program_descriptor'] extends never ? never : unknown;
};
const { project } = contracts;

test('signed build accepts only redistributable managed directories rooted at the input root', () => {
  const component = { ...project.program_descriptor.components[0], source_root: '',
    origin: { kind: 'managed', artifact_ref: 'program', redistribution_allowed: true } as const };
  assert.equal(supportsSignedBuild([component]), true);
  assert.equal(supportsSignedBuild([]), false);
  assert.equal(supportsSignedBuild([component, { ...component, source_root: 'nested' }]), false);
  assert.equal(supportsSignedBuild([{ ...component, origin: { ...component.origin, redistribution_allowed: false } }]), false);
  assert.equal(supportsSignedBuild([{ ...component, origin: { kind: 'user_provided', requirement_id: 'local', inventory_blake3: 'a'.repeat(64), instructions: 'Local' } }]), false);
});

test('clean verify rejects unapproved environment_paths and non-managed inputs', () => {
  const managedComponent = { ...project.program_descriptor.components[0], source_root: '',
    origin: { kind: 'managed', artifact_ref: 'program', redistribution_allowed: true } as const };
  assert.equal(supportsCleanVerify(project.program_descriptor), false);
  const withEnv = { ...project.program_descriptor, components: [managedComponent] };
  assert.equal(supportsCleanVerify(withEnv), false);
  const cleanDescriptor = { ...withEnv, entrypoint: { ...withEnv.entrypoint, environment_paths: [] } };
  assert.equal(supportsCleanVerify(cleanDescriptor), true);
});

test('signed build and clean verify send only compatible descriptor pairs and validate release IDs', () => {
  assert.deepEqual(signedBuildRequest(project, 'windowed', 'native'), {
    projectId: project.project_id, projectRevision: project.revision,
    buildProfileIds: ['windowed'], controlsProfileIds: ['native'], targetDirectory: null, releaseId: null, sourceUrl: null
  });
  assert.equal(signedBuildRequest(project, 'windowed', 'missing'), null);
  assert.deepEqual(verifyReleaseRequest(project, 'release-1', 'windowed', 'native'), {
    projectId: project.project_id, projectRevision: project.revision,
    buildProfileIds: ['windowed'], controlsProfileIds: ['native'], targetDirectory: null, releaseId: 'release-1', sourceUrl: null
  });
  assert.equal(verifyReleaseRequest(project, '', 'windowed', 'native'), null);
  assert.equal(verifyReleaseRequest(project, 'release-1', 'windowed', 'missing'), null);
});

test('currentRevisionReleases filters exact project and revision, and declaredProfilePairs preserves compatible pairs', () => {
  const rel1: CreatorRelease = {
    releaseId: 'release-1', projectId: project.project_id, projectRevision: project.revision,
    manifestBlake3: 'b'.repeat(64), artifactHashes: { program: 'c'.repeat(64) }, reportIds: []
  };
  const staleRev: CreatorRelease = { ...rel1, releaseId: 'release-0', projectRevision: project.revision + 1 };
  const otherProj: CreatorRelease = { ...rel1, releaseId: 'release-x', projectId: 'other-project' };
  assert.deepEqual(currentRevisionReleases([staleRev, rel1, otherProj], project), [rel1]);
  assert.deepEqual(declaredProfilePairs(project.program_descriptor), [
    { buildId: 'windowed', buildTitle: 'Windowed', controlsId: 'native', controlsTitle: 'Application defaults (no Fusion remapping)' },
    { buildId: 'fullscreen', buildTitle: 'Fullscreen + FPS', controlsId: 'native', controlsTitle: 'Application defaults (no Fusion remapping)' }
  ]);
});

test('canResumeInterruptedJob requires acceptingEvents, interrupted status and checkpoint evidence', () => {
  const checkpoint = {
    checkpointId: 'cp-1', jobId: 'job-1', boundary: 'verification_completed',
    stamp: contracts.resumeStamp, resultBlake3: 'a'.repeat(64), artifactId: null
  };
  assert.equal(canResumeInterruptedJob({ acceptingEvents: true, progress: { status: 'interrupted' }, job: { checkpoints: [checkpoint] } }), true);
  assert.equal(canResumeInterruptedJob({ acceptingEvents: false, progress: { status: 'interrupted' }, job: { checkpoints: [checkpoint] } }), false);
  assert.equal(canResumeInterruptedJob({ acceptingEvents: true, progress: { status: 'interrupted' }, job: { checkpoints: [] } }), false);
  assert.equal(canResumeInterruptedJob({ acceptingEvents: true, progress: { status: 'running' }, job: { checkpoints: [checkpoint] } }), false);
});

test('summarizePairReports keeps structure_integrity separate from clean_install and lets failure dominate', () => {
  const release: CreatorRelease = {
    releaseId: 'release-1', projectId: project.project_id, projectRevision: 1,
    manifestBlake3: 'b'.repeat(64), artifactHashes: {},
    reportIds: ['rep-struct', 'rep-wrong-hash', 'rep-other-pair', 'rep-clean-fail', 'rep-clean-pass']
  };
  const reports: Record<string, { report: CreatorReport | null }> = {
    'rep-struct': { report: { ...contracts.report, reportId: 'rep-struct', kind: 'structure_integrity', status: 'passed' } },
    'rep-wrong-hash': { report: { ...contracts.report, reportId: 'rep-wrong-hash', kind: 'clean_install', status: 'passed', releaseBlake3: 'd'.repeat(64) } },
    'rep-other-pair': { report: { ...contracts.report, reportId: 'rep-other-pair', kind: 'clean_install', status: 'passed', buildProfileId: 'fullscreen' } }
  };
  const afterStructure = summarizePairReports(release, reports, 'windowed', 'native');
  assert.equal(afterStructure.structure_integrity.status, 'passed');
  assert.equal(afterStructure.clean_install.status, 'not_tested');
  assert.equal(afterStructure.observed_launch.status, 'not_tested');
  assert.equal(afterStructure.manual_scenario.status, 'not_tested');
  assert.equal(afterStructure.https_acquisition.status, 'not_tested');

  reports['rep-clean-pass'] = { report: { ...contracts.report, reportId: 'rep-clean-pass', kind: 'clean_install', status: 'passed' } };
  reports['rep-clean-fail'] = { report: { ...contracts.report, reportId: 'rep-clean-fail', kind: 'clean_install', status: 'failed' } };
  const withFailAndPass = summarizePairReports(release, reports, 'windowed', 'native');
  assert.equal(withFailAndPass.clean_install.status, 'failed');
  assert.equal(withFailAndPass.clean_install.report?.reportId, 'rep-clean-fail');
});

test('Creator stages expose Base, Verification and Release while keeping descriptor editors unavailable', () => {
  assert.deepEqual(creatorStages, ['base', 'composition', 'profiles', 'verification', 'release']);
  for (const stage of ['base', 'verification', 'release'] as const) {
    assert.equal(creatorStageHasContent(stage), true);
  }
  for (const stage of ['composition', 'profiles'] as const) {
    assert.equal(creatorStageHasContent(stage), false);
  }
});

test('isReleaseExportReady requires every declared pair to pass structure_integrity and clean_install', () => {
  const release: CreatorRelease = {
    releaseId: 'release-1', projectId: project.project_id, projectRevision: 1,
    manifestBlake3: 'b'.repeat(64), artifactHashes: {},
    reportIds: ['r1', 'r2', 'r3', 'r4']
  };
  const reports: Record<string, { report: CreatorReport | null }> = {
    r1: { report: { ...contracts.report, reportId: 'r1', kind: 'structure_integrity', status: 'passed', buildProfileId: 'windowed' } },
    r2: { report: { ...contracts.report, reportId: 'r2', kind: 'clean_install', status: 'passed', buildProfileId: 'windowed' } },
    r3: { report: { ...contracts.report, reportId: 'r3', kind: 'structure_integrity', status: 'passed', buildProfileId: 'fullscreen' } }
  };
  const partial = isReleaseExportReady(project, release, reports);
  assert.equal(partial.ready, false);
  assert.equal(partial.hasFailedReport, false);
  assert.deepEqual(partial.missingPairs.map(pair => pair.buildId), ['fullscreen']);

  reports.r4 = { report: { ...contracts.report, reportId: 'r4', kind: 'clean_install', status: 'passed', buildProfileId: 'fullscreen' } };
  assert.equal(isReleaseExportReady(project, release, reports).ready, true);

  reports.r4 = { report: { ...contracts.report, reportId: 'r4', kind: 'clean_install', status: 'failed', buildProfileId: 'fullscreen' } };
  const failed = isReleaseExportReady(project, release, reports);
  assert.equal(failed.ready, false);
  assert.equal(failed.hasFailedReport, true);
});

test('exportReleaseRequest and verifyHttpsRequest validate inputs and compatible pairs', () => {
  assert.deepEqual(exportReleaseRequest(project, 'release-1', '  D:/exports/out  '), {
    projectId: project.project_id, projectRevision: project.revision,
    buildProfileIds: ['windowed'], controlsProfileIds: ['native'],
    releaseId: 'release-1', targetDirectory: 'D:/exports/out', sourceUrl: null
  });
  assert.equal(exportReleaseRequest(project, 'release-1', '   '), null);
  assert.equal(exportReleaseRequest(project, '', 'D:/exports/out'), null);

  assert.deepEqual(verifyHttpsRequest(project, 'release-1', ' https://example.org/source-index.json ', 'fullscreen', 'native'), {
    projectId: project.project_id, projectRevision: project.revision,
    buildProfileIds: ['fullscreen'], controlsProfileIds: ['native'],
    releaseId: 'release-1', targetDirectory: null, sourceUrl: 'https://example.org/source-index.json'
  });
  assert.equal(verifyHttpsRequest(project, 'release-1', 'http://example.org/source-index.json', 'fullscreen', 'native'), null);
  assert.equal(verifyHttpsRequest(project, 'release-1', 'https://example.org/trailing/', 'fullscreen', 'native'), null);
});
