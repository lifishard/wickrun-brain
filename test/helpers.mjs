import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const CLI = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'scripts', 'wickrun-brain.mjs');

/** A fake key that must never show up in any output. */
export const SECRET = 'sk-wbtest-Q7xR2mN9pL4vK8sJ3hG6fD1aZ5cB0eYw-tail';

/**
 * Isolated sandbox: every location the tool could touch points into a fresh temp
 * folder whose name contains spaces, so no real user file is ever read or written.
 */
export function sandbox({ configDir = true } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wb sandbox '));
  const home = path.join(root, 'home dir');
  fs.mkdirSync(home, { recursive: true });
  const env = {
    PATH: process.env.PATH || '',
    HOME: home,
    USERPROFILE: home,
    XDG_CONFIG_HOME: path.join(root, 'xdg config'),
    APPDATA: path.join(root, 'app data'),
  };
  for (const k of ['SystemRoot', 'SYSTEMROOT', 'TEMP', 'TMP', 'windir', 'ComSpec', 'PATHEXT']) if (process.env[k]) env[k] = process.env[k];
  if (configDir) env.CLAUDE_CONFIG_DIR = path.join(root, 'claude config');
  const claudeDir = configDir ? env.CLAUDE_CONFIG_DIR : path.join(home, '.claude');
  const settings = path.join(claudeDir, 'settings.json');
  const run = (args, { input, extraEnv } = {}) => {
    const r = spawnSync(process.execPath, [CLI, ...args], { env: { ...env, ...extraEnv }, input: input ?? '', encoding: 'utf8' });
    return { code: r.status, stdout: r.stdout, stderr: r.stderr, all: r.stdout + r.stderr };
  };
  const writeSettings = (content) => { fs.mkdirSync(claudeDir, { recursive: true }); fs.writeFileSync(settings, content); };
  const readSettingsBytes = () => (fs.existsSync(settings) ? fs.readFileSync(settings) : null);
  const cleanup = () => fs.rmSync(root, { recursive: true, force: true });
  return { root, home, env, claudeDir, settings, run, writeSettings, readSettingsBytes, cleanup };
}

export const sha256 = (buf) => crypto.createHash('sha256').update(buf).digest('hex');

export function addProfile(sb, name = 'work', { baseUrl = 'https://llm.example.com', model = 'gw-model-1', keyEnv = 'WB_TEST_KEY' } = {}) {
  const r = sb.run(['add', name, '--base-url', baseUrl, '--model', model, '--key-env', keyEnv]);
  if (r.code !== 0) throw new Error(`add failed: ${r.all}`);
  return r;
}

export const withKey = { extraEnv: { WB_TEST_KEY: SECRET } };
