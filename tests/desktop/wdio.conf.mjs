import path from 'node:path';

if (!process.env.FUSION_E2E_BINARY) {
  throw new Error('FUSION_E2E_BINARY is required; run the isolated build script first.');
}

export const config = {
  runner: 'local',
  specs: [path.resolve('smoke.e2e.mjs')],
  maxInstances: 1,
  capabilities: [{ browserName: 'tauri' }],
  services: [['@wdio/tauri-service', {
    driverProvider: 'embedded',
    appBinaryPath: process.env.FUSION_E2E_BINARY,
  }]],
  framework: 'mocha',
  reporters: ['spec'],
  logLevel: 'warn',
  waitforTimeout: 30000,
  connectionRetryTimeout: 120000,
  mochaOpts: { timeout: 90000 },
};
