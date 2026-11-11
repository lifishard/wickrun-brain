#!/usr/bin/env node
// wickrun-brain: see which model ("brain") Claude Code is using, switch between
// saved setups, and restore your original settings.json.
//
// Zero dependencies. Uses only node:fs, node:path, node:os and node:crypto.
// It never opens a network connection and never reads login credentials.
//
// SPDX-License-Identifier: MIT
// Copyright (c) 2026 lifishard

import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

export const VERSION = '0.1.0';
export const TOOL = 'wickrun-brain';

/** Keys inside settings.json "env" that this tool may set or remove. Nothing else is ever edited. */
export const MANAGED_KEYS = Object.freeze([
  'ANTHROPIC_BASE_URL',
  'ANTHROPIC_AUTH_TOKEN',
  'ANTHROPIC_API_KEY',
  'ANTHROPIC_MODEL',
  'ANTHROPIC_DEFAULT_OPUS_MODEL',
  'ANTHROPIC_DEFAULT_SONNET_MODEL',
  'ANTHROPIC_DEFAULT_HAIKU_MODEL',
  'ANTHROPIC_DEFAULT_FABLE_MODEL',
  'CLAUDE_CODE_SUBAGENT_MODEL',
]);
const MODEL_KEYS = ['ANTHROPIC_MODEL', 'ANTHROPIC_DEFAULT_OPUS_MODEL', 'ANTHROPIC_DEFAULT_SONNET_MODEL', 'ANTHROPIC_DEFAULT_HAIKU_MODEL', 'ANTHROPIC_DEFAULT_FABLE_MODEL', 'CLAUDE_CODE_SUBAGENT_MODEL'];
const AUTH_VARS = { bearer: 'ANTHROPIC_AUTH_TOKEN', 'x-api-key': 'ANTHROPIC_API_KEY' };

export const BACKUP_SUFFIX = '.before-wickrun-brain';
export const ABSENT_SUFFIX = '.before-wickrun-brain.absent';
/** Backup file the wickrunAI desktop app keeps next to settings.json. Read-only for us: never written or deleted. */
const DESKTOP_BACKUP_SUFFIX = '.before-wickrun';

const MODEL_RE = /^[\w./:@+-]{1,200}$/;
const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const ENV_NAME_RE = /^[A-Za-z_][A-Za-z0-9_]{0,127}$/;
const RESTART_HINT = 'Start a new Claude Code session for the change to take effect.';

export class UserError extends Error {
  constructor(message, code = 1) { super(message); this.exitCode = code; }
}

/* ------------------------------------------------------------------ paths */

export function homeDir(env = process.env, platform = process.platform) {
  const home = platform === 'win32' ? (env.USERPROFILE || env.HOME) : (env.HOME || env.USERPROFILE);
  if (!home) throw new UserError(platform === 'win32' ? 'USERPROFILE is not set; cannot find your home folder.' : 'HOME is not set; cannot find your home folder.');
  return home;
}

const pathFor = (platform) => (platform === 'win32' ? path.win32 : path.posix);

export function settingsPath(env = process.env, platform = process.platform) {
  const P = pathFor(platform);
  if (env.CLAUDE_CONFIG_DIR) {
    if (!P.isAbsolute(env.CLAUDE_CONFIG_DIR)) throw new UserError('CLAUDE_CONFIG_DIR must be an absolute path.');
    return P.join(env.CLAUDE_CONFIG_DIR, 'settings.json');
  }
  return P.join(homeDir(env, platform), '.claude', 'settings.json');
}

export function profilesPath(env = process.env, platform = process.platform) {
  const P = pathFor(platform);
  if (platform === 'win32') {
    const base = env.APPDATA && P.isAbsolute(env.APPDATA) ? env.APPDATA : P.join(homeDir(env, platform), 'AppData', 'Roaming');
    return P.join(base, TOOL, 'profiles.json');
  }
  const base = env.XDG_CONFIG_HOME && P.isAbsolute(env.XDG_CONFIG_HOME) ? env.XDG_CONFIG_HOME : P.join(homeDir(env, platform), '.config');
  return P.join(base, TOOL, 'profiles.json');
}

/** Show paths under the home folder as ~/... so output is short and does not reveal the user name. */
export function displayPath(p, env = process.env, platform = process.platform) {
  let home;
  try { home = homeDir(env, platform); } catch { return p; }
  const norm = (s) => (platform === 'win32' ? s.toLowerCase() : s);
  const sep = pathFor(platform).sep;
  const h = home.replace(/[\\/]+$/, '');
  if (norm(p).startsWith(norm(h) + sep)) return '~' + sep + p.slice(h.length + 1);
  return p;
}

/* --------------------------------------------------------------- file io */

function isPlainObject(v) { return v !== null && typeof v === 'object' && !Array.isArray(v); }

function renameWithRetry(from, to) {
  // Windows can briefly lock a file that an editor or antivirus has open.
  for (let i = 0; ; i++) {
    try { fs.renameSync(from, to); return; } catch (e) {
      if (i >= 8 || !['EPERM', 'EACCES', 'EBUSY'].includes(e.code)) throw e;
      const until = Date.now() + 25 * (i + 1);
      while (Date.now() < until) { /* short wait */ }
    }
  }
}

function tempPathFor(file) {
  return path.join(path.dirname(file), `.${path.basename(file)}.${process.pid}.${crypto.randomBytes(6).toString('hex')}.tmp`);
}

function writeTemp(file, data, mode) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = tempPathFor(file);
  const fd = fs.openSync(tmp, 'wx', mode);
  try {
    fs.writeSync(fd, data);
    if (process.platform !== 'win32') fs.fchmodSync(fd, mode);
    fs.fsyncSync(fd);
  } finally { fs.closeSync(fd); }
  return tmp;
}

/** Atomic replace: write a temp file in the same folder, then rename it over the target. */
export function writeAtomic(file, data, mode = 0o600) {
  const tmp = writeTemp(file, data, mode);
  try { renameWithRetry(tmp, file); } catch (e) { try { fs.unlinkSync(tmp); } catch { /* gone */ } throw e; }
}

/** Create a file that must not exist yet. Returns false (and writes nothing) if it already exists. */
export function createExclusive(file, data, mode = 0o600) {
  if (fs.existsSync(file)) return false;
  const tmp = writeTemp(file, data, mode);
  try {
    try { fs.linkSync(tmp, file); return true; } catch (e) {
      if (e.code === 'EEXIST') return false;
      // File systems without hard links: fall back to a checked rename.
      if (fs.existsSync(file)) return false;
      renameWithRetry(tmp, file);
      return true;
    }
  } finally { try { fs.unlinkSync(tmp); } catch { /* renamed or linked */ } }
}

/** Read and validate settings.json. Throws UserError (changing nothing) if it is unusable. */
export function readSettings(file) {
  let raw;
  try { raw = fs.readFileSync(file); } catch (e) {
    if (e.code === 'ENOENT') return { file, exists: false, raw: null, data: {}, eol: '\n', bom: false, trailingNewline: true };
    throw new UserError(`Cannot read ${file}: ${e.code || e.message}`);
  }
  let text = raw.toString('utf8');
  const bom = text.charCodeAt(0) === 0xfeff;
  if (bom) text = text.slice(1);
  let data;
  try { data = JSON.parse(text); } catch {
    throw new UserError(`${file} is not valid JSON. Nothing was changed. Fix the file (or run "${TOOL} restore" if this tool changed it) and try again.`);
  }
  if (!isPlainObject(data)) throw new UserError(`${file} must contain a JSON object. Nothing was changed.`);
  if (data.env !== undefined && !isPlainObject(data.env)) throw new UserError(`"env" in ${file} is not an object. Nothing was changed.`);
  return { file, exists: true, raw, data, eol: text.includes('\r\n') ? '\r\n' : '\n', bom, trailingNewline: /\n$/.test(text) };
}

export function serializeSettings(s, data) {
  let out = JSON.stringify(data, null, 2);
  if (s.trailingNewline) out += '\n';
  if (s.eol === '\r\n') out = out.replace(/\n/g, '\r\n');
  return Buffer.from((s.bom ? '﻿' : '') + out, 'utf8');
}

/* -------------------------------------------------------------- profiles */

export function readProfiles(file) {
  let text;
  try { text = fs.readFileSync(file, 'utf8'); } catch (e) {
    if (e.code === 'ENOENT') return { version: 1, profiles: {} };
    throw new UserError(`Cannot read ${file}: ${e.code || e.message}`);
  }
  let data;
  try { data = JSON.parse(text); } catch { throw new UserError(`${file} is not valid JSON. Fix or delete it.`); }
  if (!isPlainObject(data) || !isPlainObject(data.profiles)) throw new UserError(`${file} has an unexpected format. Fix or delete it.`);
  return data;
}

function writeProfiles(file, data) {
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  writeAtomic(file, JSON.stringify(data, null, 2) + '\n', 0o600);
}

/* ------------------------------------------------------------ validation */

export function validateBaseUrl(value) {
  if (typeof value !== 'string' || !value) throw new UserError('--base-url is required.', 2);
  let u;
  try { u = new URL(value); } catch { throw new UserError('The base URL is not a valid URL.', 2); }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') throw new UserError('The base URL must start with https:// or http://.', 2);
  if (u.username || u.password) throw new UserError('The base URL must not contain a user name or password. Use --key-env or --key-stdin for the key.', 2);
  if (value.includes('?') || value.includes('#')) throw new UserError('The base URL must not contain a query (?) or fragment (#).', 2);
  if (!u.hostname) throw new UserError('The base URL has no host.', 2);
  return value;
}
export function validateModel(value) {
  if (typeof value !== 'string' || !MODEL_RE.test(value)) throw new UserError('--model must be 1-200 characters of letters, digits, and . / : @ + - _', 2);
  return value;
}
export function validateName(value) {
  if (typeof value !== 'string' || !NAME_RE.test(value)) throw new UserError('Profile names use letters, digits, ".", "_" and "-" (max 64, starting with a letter or digit).', 2);
  return value;
}
function validateKey(key) {
  if (!key) throw new UserError('The key is empty.');
  if (key.length > 4096) throw new UserError('The key is longer than 4096 characters.');
  if (/[\s\u0000-\u001f\u007f]/.test(key)) throw new UserError('The key contains spaces or control characters.');
  return key;
}

function isLoopback(host) { return ['localhost', '127.0.0.1', '::1', '[::1]'].includes(host) || /^127\./.test(host); }

export function hostOf(url) {
  try { return new URL(url).host || null; } catch { return url ? '(invalid URL)' : null; }
}

/** Never print a secret: at most the last 4 characters, and only for long keys. */
export function maskSecret(value) {
  if (typeof value !== 'string' || !value) return null;
  return value.length >= 20 ? `…${value.slice(-4)}` : 'set';
}

/* ---------------------------------------------------- state & detection */

/**
 * The wickrunAI desktop app can take over the same keys. It does not write a marker
 * into settings.json, so we recognise the shape it leaves behind:
 *  - "route" takeover: base URL on this machine (its local proxy), nonessential
 *    traffic switched off, and the subagent model pinned to the main model.
 *  - "subscription" takeover: forceLoginMethod "claudeai" with its backup file present.
 */
export function detectDesktop(data, file) {
  const env = isPlainObject(data.env) ? data.env : {};
  let hostname = null;
  try { hostname = new URL(env.ANTHROPIC_BASE_URL).hostname; } catch { /* not set */ }
  if (hostname && isLoopback(hostname) && env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC === '1'
      && env.ANTHROPIC_MODEL && env.CLAUDE_CODE_SUBAGENT_MODEL === env.ANTHROPIC_MODEL) {
    return { detected: true, kind: 'route' };
  }
  if (data.forceLoginMethod === 'claudeai' && fs.existsSync(file + DESKTOP_BACKUP_SUFFIX)) {
    return { detected: true, kind: 'subscription' };
  }
  return { detected: false, kind: null };
}

function backupState(file) {
  const content = fs.existsSync(file + BACKUP_SUFFIX);
  const absent = fs.existsSync(file + ABSENT_SUFFIX);
  return { exists: content || absent, content, absent, path: content ? file + BACKUP_SUFFIX : absent ? file + ABSENT_SUFFIX : null };
}

function activeProfileName(env, profiles) {
  for (const [name, p] of Object.entries(profiles.profiles)) {
    if (p.baseUrl === env.ANTHROPIC_BASE_URL && p.model === env.ANTHROPIC_MODEL) return name;
  }
  return null;
}

export function computeStatus(procEnv = process.env) {
  const file = settingsPath(procEnv);
  const s = readSettings(file);
  const env = isPlainObject(s.data.env) ? s.data.env : {};
  const keysSet = MANAGED_KEYS.filter((k) => Object.prototype.hasOwnProperty.call(env, k));
  const desktop = detectDesktop(s.data, file);
  let profiles = { profiles: {} };
  try { profiles = readProfiles(profilesPath(procEnv)); } catch { /* status still works */ }
  let mode = 'official';
  if (desktop.detected) mode = 'wickrunai-desktop';
  else if (keysSet.length) mode = 'custom';
  const credVar = env.ANTHROPIC_AUTH_TOKEN !== undefined ? 'ANTHROPIC_AUTH_TOKEN' : env.ANTHROPIC_API_KEY !== undefined ? 'ANTHROPIC_API_KEY' : null;
  const b = backupState(file);
  return {
    tool: TOOL,
    version: VERSION,
    settingsFile: file,
    settingsExists: s.exists,
    mode,
    modeLabel: { official: 'official login (no override)', custom: 'custom endpoint', 'wickrunai-desktop': 'managed by wickrunAI desktop' }[mode],
    overridden: keysSet.length > 0,
    keysSet,
    baseUrlHost: hostOf(env.ANTHROPIC_BASE_URL),
    model: typeof env.ANTHROPIC_MODEL === 'string' ? env.ANTHROPIC_MODEL : null,
    credential: credVar ? { variable: credVar, masked: maskSecret(String(env[credVar])) } : null,
    profile: mode === 'custom' ? activeProfileName(env, profiles) : null,
    backup: { exists: b.exists, path: b.path },
    desktop,
  };
}

/* -------------------------------------------------------------- commands */

function ensureBackup(s) {
  const b = backupState(s.file);
  if (b.exists) return false;
  if (s.exists) return createExclusive(s.file + BACKUP_SUFFIX, s.raw, 0o600);
  return createExclusive(s.file + ABSENT_SUFFIX, Buffer.alloc(0), 0o600);
}

function refuseIfDesktop(data, file, force, action) {
  const d = detectDesktop(data, file);
  if (d.detected && !force) {
    throw new UserError(`Claude Code is currently managed by the wickrunAI desktop app. Switch or restore from the wickrunAI app instead, or run "${TOOL} ${action} --force" to override it anyway.`);
  }
  return d;
}

/** Remove every managed key from a copy of the settings data. */
function withoutManaged(data) {
  const next = { ...data };
  if (isPlainObject(data.env)) {
    const env = { ...data.env };
    for (const k of MANAGED_KEYS) delete env[k];
    if (Object.keys(env).length) next.env = env; else delete next.env;
  }
  return next;
}

function applyData(s, next) {
  const bytes = serializeSettings(s, next);
  if (s.exists && Buffer.compare(bytes, s.raw) === 0) return { changed: false };
  const createdBackup = ensureBackup(s);
  writeAtomic(s.file, bytes, 0o600);
  return { changed: true, createdBackup };
}

export function cmdUse(name, opts, procEnv, out) {
  validateName(name);
  const file = settingsPath(procEnv);
  const s = readSettings(file);
  const d = refuseIfDesktop(s.data, file, opts.force, `use ${name}`);
  const pfile = profilesPath(procEnv);
  const profiles = readProfiles(pfile);
  const p = profiles.profiles[name];
  if (!p) throw new UserError(`No profile named "${name}". Run "${TOOL} list" to see saved profiles.`);
  validateBaseUrl(p.baseUrl);
  validateModel(p.model);
  let key;
  if (p.keyEnv) {
    key = procEnv[p.keyEnv];
    if (!key) throw new UserError(`Profile "${name}" reads its key from the environment variable ${p.keyEnv}, which is not set here. Set it in the terminal you start Claude Code from, then try again.`);
  } else if (typeof p.key === 'string') key = p.key;
  else throw new UserError(`Profile "${name}" has no key. Add it again with --key-env or --key-stdin.`);
  validateKey(key);
  const next = withoutManaged(s.data);
  const env = { ...(isPlainObject(next.env) ? next.env : {}) };
  env.ANTHROPIC_BASE_URL = p.baseUrl;
  env[AUTH_VARS[p.auth] || AUTH_VARS.bearer] = key;
  for (const k of MODEL_KEYS) env[k] = p.model;
  next.env = env;
  const r = applyData(s, next);
  if (!r.changed) { out(`Profile "${name}" is already active in ${displayPath(file, procEnv)}. Nothing changed.`); return; }
  if (d.detected) out('Overrode the wickrunAI desktop app (--force). The app may show a stale state until you switch there.');
  out(`Switched Claude Code to profile "${name}": ${hostOf(p.baseUrl)}, model ${p.model}.`);
  out(`Updated ${displayPath(file, procEnv)}${r.createdBackup ? ' (original saved for "restore")' : ''}.`);
  out(RESTART_HINT);
}

export function cmdOfficial(opts, procEnv, out) {
  const file = settingsPath(procEnv);
  const s = readSettings(file);
  refuseIfDesktop(s.data, file, opts.force, 'official');
  const env = isPlainObject(s.data.env) ? s.data.env : {};
  if (!MANAGED_KEYS.some((k) => Object.prototype.hasOwnProperty.call(env, k))) {
    out('Already using the official login (no override). Nothing changed.');
    return;
  }
  applyData(s, withoutManaged(s.data));
  out(`Removed the endpoint, key and model overrides from ${displayPath(file, procEnv)}.`);
  out('Claude Code will use your official login again.');
  out(RESTART_HINT);
}

export function cmdRestore(opts, procEnv, out) {
  const file = settingsPath(procEnv);
  const b = backupState(file);
  if (!b.exists) { out(`No backup found next to ${displayPath(file, procEnv)}. ${TOOL} has not changed it, so there is nothing to restore.`); return; }
  if (b.content && b.absent) throw new UserError(`Both ${path.basename(file + BACKUP_SUFFIX)} and ${path.basename(file + ABSENT_SUFFIX)} exist. Remove the one that is wrong, then run restore again.`);
  // Restore does not need the current file to be valid JSON, but if it is, respect a desktop takeover.
  let current = null;
  try { current = readSettings(file); } catch { /* broken file: restoring the original is the fix */ }
  if (current && current.exists) refuseIfDesktop(current.data, file, opts.force, 'restore');
  if (b.content) {
    const original = fs.readFileSync(file + BACKUP_SUFFIX);
    writeAtomic(file, original, 0o600);
    fs.unlinkSync(file + BACKUP_SUFFIX);
    out(`settings.json restored, byte-identical to the original (sha256 ${crypto.createHash('sha256').update(original).digest('hex').slice(0, 12)}).`);
  } else {
    try { fs.unlinkSync(file); } catch (e) { if (e.code !== 'ENOENT') throw e; }
    fs.unlinkSync(file + ABSENT_SUFFIX);
    out('settings.json did not exist before; removed it again.');
  }
  out('Backup deleted.');
  out(RESTART_HINT);
}

export async function cmdAdd(name, opts, procEnv, out, err, readKeyFromStdin) {
  validateName(name);
  const baseUrl = validateBaseUrl(opts['base-url']);
  const model = validateModel(opts.model);
  const auth = opts.auth || 'bearer';
  if (!AUTH_VARS[auth]) throw new UserError('--auth must be "bearer" (ANTHROPIC_AUTH_TOKEN) or "x-api-key" (ANTHROPIC_API_KEY).', 2);
  if (opts['key-env'] && opts['key-stdin']) throw new UserError('Use either --key-env or --key-stdin, not both.', 2);
  if (!opts['key-env'] && !opts['key-stdin']) throw new UserError('A key is required: use --key-env <VAR_NAME> or --key-stdin. Keys are never accepted as command-line arguments.', 2);
  const pfile = profilesPath(procEnv);
  const profiles = readProfiles(pfile);
  if (profiles.profiles[name] && !opts.force) throw new UserError(`A profile named "${name}" already exists. Use --force to replace it.`);
  const entry = { baseUrl, model, auth };
  if (opts['key-env']) {
    if (!ENV_NAME_RE.test(opts['key-env'])) throw new UserError('--key-env takes the NAME of an environment variable, for example MY_GATEWAY_KEY.', 2);
    entry.keyEnv = opts['key-env'];
    if (!procEnv[entry.keyEnv]) err(`Note: ${entry.keyEnv} is not set in this shell. It must be set when you run "${TOOL} use ${name}".`);
  } else {
    entry.key = validateKey(await readKeyFromStdin());
  }
  const u = new URL(baseUrl);
  if (u.protocol === 'http:' && !isLoopback(u.hostname)) err('Warning: this base URL uses plain http to another machine; the key would travel unencrypted.');
  profiles.version = 1;
  profiles.profiles[name] = entry;
  writeProfiles(pfile, profiles);
  out(`Saved profile "${name}": ${hostOf(baseUrl)}, model ${model}, key ${entry.keyEnv ? `from $${entry.keyEnv}` : 'stored in the profiles file'}.`);
  out(`Profiles file: ${displayPath(pfile, procEnv)}`);
  out(`Apply it with: ${TOOL} use ${name}`);
}

export function cmdRemove(name, procEnv, out) {
  validateName(name);
  const pfile = profilesPath(procEnv);
  const profiles = readProfiles(pfile);
  if (!profiles.profiles[name]) throw new UserError(`No profile named "${name}".`);
  delete profiles.profiles[name];
  writeProfiles(pfile, profiles);
  out(`Removed profile "${name}". settings.json was not changed.`);
}

export function listData(procEnv = process.env) {
  const pfile = profilesPath(procEnv);
  const profiles = readProfiles(pfile);
  let active = null;
  try {
    const s = readSettings(settingsPath(procEnv));
    if (isPlainObject(s.data.env)) active = activeProfileName(s.data.env, profiles);
  } catch { /* list still works */ }
  return {
    tool: TOOL,
    version: VERSION,
    profilesFile: pfile,
    profiles: Object.entries(profiles.profiles).sort(([a], [b]) => a.localeCompare(b)).map(([name, p]) => ({
      name,
      baseUrlHost: hostOf(p.baseUrl),
      model: p.model ?? null,
      auth: p.auth || 'bearer',
      keySource: p.keyEnv ? 'env' : 'stored',
      ...(p.keyEnv ? { keyEnv: p.keyEnv } : {}),
      active: name === active,
    })),
  };
}

function pad(s, n) { s = String(s); return s.length >= n ? s : s + ' '.repeat(n - s.length); }

export function formatStatus(st, procEnv = process.env) {
  const rows = [['settings file', displayPath(st.settingsFile, procEnv) + (st.settingsExists ? '' : ' (does not exist)')]];
  let mode = st.modeLabel;
  if (st.mode === 'custom' && st.profile) mode += ` (profile "${st.profile}")`;
  if (st.mode === 'wickrunai-desktop') mode += st.desktop.kind === 'subscription' ? ' (official subscription login)' : ' (local route)';
  rows.push(['mode', mode]);
  if (!st.overridden) rows.push(['overrides', 'none']);
  else {
    if (st.baseUrlHost) rows.push(['base URL host', st.baseUrlHost]);
    rows.push(['model', st.model || '(not set)']);
    rows.push(['key', st.credential ? `${st.credential.variable} ${st.credential.masked}` : '(none)']);
  }
  if (st.backup.exists) rows.push(['backup', st.backup.path.endsWith(ABSENT_SUFFIX) ? '(settings.json did not exist before)' : path.basename(st.backup.path)]);
  const lines = rows.map(([k, v]) => `  ${pad(k, 16)}${v}`);
  if (st.mode === 'wickrunai-desktop') lines.push('', '  Switch or restore from the wickrunAI app.');
  else if (st.backup.exists) lines.push('', `  Undo every change with: ${TOOL} restore`);
  return lines.join('\n');
}

export function formatList(data, procEnv = process.env) {
  if (!data.profiles.length) return `No saved profiles (${displayPath(data.profilesFile, procEnv)}).\nAdd one from your own terminal: ${TOOL} add <name> --base-url <url> --model <id> --key-env <VAR_NAME>`;
  const w1 = Math.max(4, ...data.profiles.map((p) => p.name.length)) + 2;
  const w2 = Math.max(4, ...data.profiles.map((p) => String(p.baseUrlHost).length)) + 2;
  const w3 = Math.max(5, ...data.profiles.map((p) => String(p.model).length)) + 2;
  const lines = [`Saved profiles (${displayPath(data.profilesFile, procEnv)}):`];
  for (const p of data.profiles) {
    lines.push(`${p.active ? '* ' : '  '}${pad(p.name, w1)}${pad(p.baseUrlHost, w2)}${pad(p.model, w3)}key: ${p.keySource === 'env' ? `$${p.keyEnv}` : 'stored'}`);
  }
  if (data.profiles.some((p) => p.active)) lines.push('(* = active in settings.json)');
  return lines.join('\n');
}

export const HELP = `${TOOL} ${VERSION}
See which model ("brain") Claude Code is using, switch between saved setups,
and restore your original config.

Usage:
  ${TOOL} status [--json]       Show what settings.json points Claude Code at
  ${TOOL} list [--json]         List saved profiles (keys are never shown)
  ${TOOL} add <name> --base-url <url> --model <id> (--key-env <VAR> | --key-stdin)
                                 [--auth bearer|x-api-key] [--force]
  ${TOOL} use <name> [--force]  Apply a saved profile to settings.json
  ${TOOL} official [--force]    Remove all overrides (back to the official login)
  ${TOOL} restore [--force]     Put settings.json back exactly as it was before
  ${TOOL} remove <name>         Delete a saved profile
  ${TOOL} --help | --version

Keys are read from an environment variable (--key-env) or stdin (--key-stdin),
never from the command line. Run "add" in your own terminal, not in a chat.
The tool makes no network requests.`;

/* ------------------------------------------------------------------ cli */

const BOOL_FLAGS = new Set(['json', 'help', 'version', 'force', 'key-stdin']);
const VALUE_FLAGS = new Set(['base-url', 'model', 'key-env', 'auth']);

export function parseArgs(argv) {
  const pos = [];
  const opts = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '-h') { opts.help = true; continue; }
    if (a === '-v' || a === '-V') { opts.version = true; continue; }
    if (a.startsWith('--')) {
      const eq = a.indexOf('=');
      const name = eq < 0 ? a.slice(2) : a.slice(2, eq);
      if (['key', 'api-key', 'token', 'auth-token'].includes(name)) throw new UserError('Keys are never accepted on the command line (they would end up in your shell history). Use --key-env <VAR_NAME> or --key-stdin.', 2);
      if (BOOL_FLAGS.has(name)) { if (eq >= 0) throw new UserError(`--${name} takes no value.`, 2); opts[name] = true; continue; }
      if (VALUE_FLAGS.has(name)) {
        const v = eq >= 0 ? a.slice(eq + 1) : argv[++i];
        if (v === undefined || (eq < 0 && v.startsWith('--'))) throw new UserError(`--${name} needs a value.`, 2);
        opts[name] = v;
        continue;
      }
      throw new UserError(`Unknown option: ${a}. See "${TOOL} --help".`, 2);
    }
    if (a.startsWith('-') && a !== '-') throw new UserError(`Unknown option: ${a}. See "${TOOL} --help".`, 2);
    pos.push(a);
  }
  return { pos, opts };
}

async function readStdinKey() {
  const stdin = process.stdin;
  if (stdin.isTTY && typeof stdin.setRawMode === 'function') {
    process.stderr.write('Paste the key and press Enter (input is hidden): ');
    return await new Promise((resolve, reject) => {
      let buf = '';
      stdin.setRawMode(true);
      stdin.resume();
      stdin.setEncoding('utf8');
      const done = (fn, v) => { stdin.setRawMode(false); stdin.pause(); stdin.removeListener('data', onData); process.stderr.write('\n'); fn(v); };
      const onData = (chunk) => {
        for (const ch of chunk) {
          if (ch === '\r' || ch === '\n') return done(resolve, buf);
          if (ch === '\u0003') return done(reject, new UserError('Cancelled.'));
          if (ch === '\u007f' || ch === '\b') buf = buf.slice(0, -1);
          else if (ch >= ' ') buf += ch;
        }
      };
      stdin.on('data', onData);
    });
  }
  const chunks = [];
  for await (const c of stdin) chunks.push(typeof c === 'string' ? Buffer.from(c) : c);
  return Buffer.concat(chunks).toString('utf8').replace(/^﻿/, '').trim();
}

export async function main(argv = process.argv.slice(2), procEnv = process.env) {
  const out = (s) => process.stdout.write(s + '\n');
  const err = (s) => process.stderr.write(s + '\n');
  try {
    const { pos, opts } = parseArgs(argv);
    if (opts.version) { out(VERSION); return 0; }
    const cmd = pos[0];
    if (opts.help || !cmd || cmd === 'help') { out(HELP); return cmd || opts.help ? 0 : 2; }
    const allowed = {
      status: ['json'], list: ['json'], add: ['base-url', 'model', 'key-env', 'key-stdin', 'auth', 'force'],
      use: ['force'], official: ['force'], restore: ['force'], remove: [],
    }[cmd];
    if (!allowed) throw new UserError(`Unknown command: ${cmd}. See "${TOOL} --help".`, 2);
    for (const k of Object.keys(opts)) if (!allowed.includes(k)) throw new UserError(`--${k} is not valid for "${cmd}".`, 2);
    const needName = ['add', 'use', 'remove'].includes(cmd);
    if (needName && pos.length !== 2) throw new UserError(`Usage: ${TOOL} ${cmd} <name>${cmd === 'add' ? ' --base-url <url> --model <id> (--key-env <VAR> | --key-stdin)' : ''}`, 2);
    if (!needName && pos.length !== 1) throw new UserError(`"${cmd}" takes no extra arguments.`, 2);
    switch (cmd) {
      case 'status': {
        const st = computeStatus(procEnv);
        out(opts.json ? JSON.stringify(st, null, 2) : formatStatus(st, procEnv));
        break;
      }
      case 'list': {
        const d = listData(procEnv);
        out(opts.json ? JSON.stringify(d, null, 2) : formatList(d, procEnv));
        break;
      }
      case 'add': await cmdAdd(pos[1], opts, procEnv, out, err, readStdinKey); break;
      case 'use': cmdUse(pos[1], opts, procEnv, out); break;
      case 'official': cmdOfficial(opts, procEnv, out); break;
      case 'restore': cmdRestore(opts, procEnv, out); break;
      case 'remove': cmdRemove(pos[1], procEnv, out); break;
    }
    return 0;
  } catch (e) {
    if (e instanceof UserError) { err(`${TOOL}: ${e.message}`); return e.exitCode; }
    err(`${TOOL}: unexpected error: ${e && e.code ? e.code : ''} ${e && e.message ? e.message : e}`.trim());
    return 1;
  }
}

const invokedDirectly = (() => {
  try { return process.argv[1] && fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url)); } catch { return false; }
})();
if (invokedDirectly) {
  main().then((code) => { process.exitCode = code; });
}
