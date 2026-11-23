import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createCreatorBridge, creatorApi, fusionInputApi } from './creatorApi.ts';
import { CREATOR_COMMANDS, INPUT_COMMANDS } from '../types/fusionCreator.ts';
import type { CreatorTransport } from './creatorApi.ts';

test('every frozen API method invokes the exact command and sole request envelope', async () => {
  const calls: unknown[] = [];
  const transport: CreatorTransport = {
    invoke: async <T>(command: string, args?: { request: unknown }): Promise<T> => {
      calls.push({ command, args });
      return undefined as T;
    },
    listen: async () => () => undefined
  };
  const bridge = createCreatorBridge(transport);
  const request = { identity: 'request-is-passed-unchanged' };
  const creatorMethods = ['projectList', 'projectOpen', 'projectSave', 'scan', 'classify', 'build', 'verify', 'export', 'verifyHttps', 'jobGet', 'jobCancel', 'jobResume', 'releaseList', 'reportGet', 'privateBindingsSave'];
  const inputMethods = ['devices', 'sessionStart', 'sessionStop', 'sessionGet', 'bindingsSave', 'releaseForLaunch'];
  for (const [api, methods, commands] of [
    [bridge.creatorApi, creatorMethods, CREATOR_COMMANDS],
    [bridge.fusionInputApi, inputMethods, INPUT_COMMANDS]
  ] as const) {
    for (const [index, method] of methods.entries()) {
      const noRequest = method === 'projectList' || method === 'devices';
      await (api as unknown as Record<string, (value?: unknown) => Promise<unknown>>)[method](noRequest ? undefined : request);
      assert.deepEqual(calls.at(-1), { command: commands[index], args: noRequest ? undefined : { request } });
    }
  }
});

test('browser production APIs reject action_unavailable with the precise command', async () => {
  await assert.rejects(creatorApi.projectList(), { code: 'action_unavailable', details: { command: 'fusion_creator_project_list' } });
  await assert.rejects(fusionInputApi.devices(), { code: 'action_unavailable', details: { command: 'fusion_input_devices' } });
});

test('private bindings use their own local command and revision-only receipt', async () => {
  const request = {
    projectRevision: 2, expectedBindingsRevision: null,
    bindings: { projectId: 'project', componentPaths: { runtime: 'C:/local/runtime' }, signingKeyRef: null }
  };
  const bridge = createCreatorBridge({
    invoke: async <T>(command: string, args?: { request: unknown }) => {
      assert.equal(command, 'fusion_creator_private_bindings_save');
      assert.deepEqual(args, { request });
      return { revision: 0 } as T;
    },
    listen: async () => () => undefined
  });
  assert.deepEqual(await bridge.creatorApi.privateBindingsSave(request), { revision: 0 });
  await assert.rejects(creatorApi.privateBindingsSave(request), { code: 'action_unavailable', details: { command: 'fusion_creator_private_bindings_save' } });
});
