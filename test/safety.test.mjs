// One block per safety rule in the README ("Safety rules").
import { test, describe, after, mock } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { sandbox, sha256, addProfile, withKey, SECRET, CLI } from './helpers.mjs';
import * as wb from '../scripts/wickrun-brain.mjs';

const ORIGINAL = Buffer.from('{\n    "theme": "dark",\n    "env": { "MY_OWN_VAR": "keep me", "ANTHROPIC_MODEL": "user-model" },\n    "unicode": "\u4e2d\u6587 \u00e9",\n    "permissions": {"allow": ["Bash(ls)"]}\n}', 'utf8'); // 4-space indent, no trailing newline

describe('rule 1: backup before the first change, never overwritten, byte-exact restore', () => {
  const sbs = [];
  after(() => sbs.forEach((s) => s.cleanup()));

  test('first change saves the original bytes; restore is byte-identical and deletes the backup', () => {
    const sb = sandbox(); sbs.push(sb);
    sb.writeSettings(ORIGINAL);
    const before = sha256(ORIGINAL);
    addProfile(sb);
    assert.equal(sb.run(['use', 'work'], withKey).code, 0);
    const backup = sb.settings + wb.BACKUP_SUFFIX;
    assert.equal(sha256(fs.readFileSync(backup)), before, 'backup holds the original bytes');
    assert.notEqual(sha256(sb.readSettingsBytes()), before);
    assert.equal(sb.run(['official']).code, 0);
    const r = sb.run(['restore']);
    assert.equal(r.code, 0, r.all);
    assert.match(r.stdout, /byte-identical/);
    assert.equal(sha256(sb.readSettingsBytes()), before);
    assert.ok(!fs.existsSync(backup), 'backup removed after restore');
  });

  test('an existing backup is never overwritten', () => {
    const sb = sandbox(); sbs.push(sb);
    sb.writeSettings('{"a":1}\n');
    const backup = sb.settings + wb.BACKUP_SUFFIX;
    fs.writeFileSync(backup, 'EARLIER BACKUP');
    addProfile(sb);
    assert.equal(sb.run(['use', 'work'], withKey).code, 0);
    assert.equal(fs.readFileSync(backup, 'utf8'), 'EARLIER BACKUP');
    addProfile(sb, 'other', { model: 'other-model' });
    assert.equal(sb.run(['use', 'other'], withKey).code, 0);
    assert.equal(fs.readFileSync(backup, 'utf8'), 'EARLIER BACKUP');
  });

  test('second change keeps the first backup (still the true original)', () => {
    const sb = sandbox(); sbs.push(sb);
    sb.writeSettings(ORIGINAL);
    addProfile(sb); addProfile(sb, 'b', { model: 'm-b' });
    sb.run(['use', 'work'], withKey);
    sb.run(['use', 'b'], withKey);
    assert.equal(sha256(fs.readFileSync(sb.settings + wb.BACKUP_SUFFIX)), sha256(ORIGINAL));
    sb.run(['restore']);
    assert.equal(sha256(sb.readSettingsBytes()), sha256(ORIGINAL));
  });

  test('settings.json that did not exist: restore removes it again', () => {
    const sb = sandbox(); sbs.push(sb);
    addProfile(sb);
    assert.equal(sb.run(['use', 'work'], withKey).code, 0);
    assert.ok(fs.existsSync(sb.settings));
    assert.ok(fs.existsSync(sb.settings + wb.ABSENT_SUFFIX));
    const r = sb.run(['restore']);
    assert.equal(r.code, 0, r.all);
    assert.ok(!fs.existsSync(sb.settings));
    assert.ok(!fs.existsSync(sb.settings + wb.ABSENT_SUFFIX));
  });

  test('restore with no backup changes nothing', () => {
    const sb = sandbox(); sbs.push(sb);
    sb.writeSettings(ORIGINAL);
    const r = sb.run(['restore']);
    assert.equal(r.code, 0);
    assert.match(r.stdout, /nothing to restore/);
    assert.equal(sha256(sb.readSettingsBytes()), sha256(ORIGINAL));
  });

  test('a no-op command does not create a backup', () => {
    const sb = sandbox(); sbs.push(sb);
    sb.writeSettings('{"theme":"dark"}\n');
    assert.equal(sb.run(['official']).code, 0);
    assert.ok(!fs.existsSync(sb.settings + wb.BACKUP_SUFFIX));
  });

  test('restore works even if settings.json was later broken by hand', () => {
    const sb = sandbox(); sbs.push(sb);
    sb.writeSettings(ORIGINAL);
    addProfile(sb);
    sb.run(['use', 'work'], withKey);
    fs.writeFileSync(sb.settings, '{ broken');
    assert.equal(sb.run(['restore']).code, 0);
    assert.equal(sha256(sb.readSettingsBytes()), sha256(ORIGINAL));
  });
});

describe('rule 2: atomic writes and 0600', () => {
  const sbs = [];
  after(() => sbs.forEach((s) => s.cleanup()));

  test('writeAtomic renames a temp file from the same folder over the target', () => {
    const sb = sandbox(); sbs.push(sb);
    const target = path.join(sb.root, 'dir with space', 'f.json');
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, 'old');
    const spy = mock.method(fs, 'renameSync');
    try { wb.writeAtomic(target, 'new'); } finally { spy.mock.restore(); }
    assert.equal(spy.mock.callCount(), 1);
    const [from, to] = spy.mock.calls[0].arguments;
    assert.equal(to, target);
    assert.equal(path.dirname(from), path.dirname(target));
    assert.notEqual(from, target);
    assert.equal(fs.readFileSync(target, 'utf8'), 'new');
    assert.deepEqual(fs.readdirSync(path.dirname(target)), ['f.json'], 'no temp file left behind');
  });

  test('settings.json, backup and profiles file are 0600 (POSIX)', { skip: process.platform === 'win32' && 'POSIX modes only' }, () => {
    const sb = sandbox(); sbs.push(sb);
    sb.writeSettings('{}\n');
    fs.chmodSync(sb.settings, 0o644);
    assert.equal(sb.run(['add', 'k', '--base-url', 'https://a.example', '--model', 'm', '--key-stdin'], { input: SECRET }).code, 0);
    assert.equal(sb.run(['use', 'k']).code, 0);
    const mode = (p) => fs.statSync(p).mode & 0o777;
    assert.equal(mode(sb.settings), 0o600);
    assert.equal(mode(sb.settings + wb.BACKUP_SUFFIX), 0o600);
    assert.equal(mode(wb.profilesPath(sb.env)), 0o600);
  });

  test('no temp files are left in the settings folder', () => {
    const sb = sandbox(); sbs.push(sb);
    sb.writeSettings('{}\n');
    addProfile(sb);
    sb.run(['use', 'work'], withKey); sb.run(['official']); sb.run(['restore']);
    assert.deepEqual(fs.readdirSync(sb.claudeDir).filter((f) => f.endsWith('.tmp')), []);
  });
});

describe('rule 3: refuse invalid settings.json, change nothing', () => {
  const cases = [
    ['not JSON', '{ "theme": "dark", '],
    ['empty file', ''],
    ['top-level array', '[1,2]'],
    ['env is a string', '{"env":"x"}'],
    ['env is an array', '{"env":[]}'],
    ['env is null', '{"env":null}'],
  ];
  for (const [label, content] of cases) {
    test(label, () => {
      const sb = sandbox();
      try {
        sb.writeSettings(content);
        addProfile(sb);
        for (const args of [['use', 'work'], ['official']]) {
          const r = sb.run(args, withKey);
          assert.equal(r.code, 1, `${args[0]}: ${r.all}`);
          assert.match(r.stderr, /Nothing was changed|not valid JSON|must contain a JSON object|not an object/);
          assert.equal(sb.readSettingsBytes().toString('utf8'), content);
          assert.ok(!fs.existsSync(sb.settings + wb.BACKUP_SUFFIX));
        }
        assert.equal(sb.run(['status']).code, 1);
      } finally { sb.cleanup(); }
    });
  }
});

describe('rule 4: unrelated content is preserved', () => {
  test('unknown keys, other env vars, unicode, trailing newline', () => {
    const sb = sandbox();
    try {
      const orig = '{\n  "theme": "dark",\n  "zeta": {"nested": [1, 2, {"x": null}]},\n  "env": {"MY_OWN_VAR": "keep me", "ANTHROPIC_MODEL": "old"},\n  "name": "\u00e9\u4e2d"\n}\n';
      sb.writeSettings(orig);
      addProfile(sb);
      sb.run(['use', 'work'], withKey);
      const after1 = sb.readSettingsBytes().toString('utf8');
      assert.ok(after1.endsWith('}\n'));
      const d = JSON.parse(after1);
      assert.equal(d.theme, 'dark');
      assert.deepEqual(d.zeta, { nested: [1, 2, { x: null }] });
      assert.equal(d.name, '\u00e9\u4e2d');
      assert.equal(d.env.MY_OWN_VAR, 'keep me');
      assert.deepEqual(Object.keys(d), ['theme', 'zeta', 'env', 'name'], 'key order kept');
      assert.match(after1, /^\{\n {2}"theme"/, '2-space indent');
      sb.run(['official']);
      const d2 = JSON.parse(sb.readSettingsBytes().toString('utf8'));
      assert.deepEqual(d2.env, { MY_OWN_VAR: 'keep me' });
      for (const k of wb.MANAGED_KEYS) assert.ok(!(k in d2.env));
    } finally { sb.cleanup(); }
  });

  test('no trailing newline stays without one; CRLF stays CRLF; env removed when only managed keys were in it', () => {
    const sb = sandbox();
    try {
      sb.writeSettings('{"a": 1,\r\n "env": {"ANTHROPIC_MODEL": "x"}}');
      sb.run(['official']);
      const t = sb.readSettingsBytes().toString('utf8');
      assert.equal(t, '{\r\n  "a": 1\r\n}');
    } finally { sb.cleanup(); }
  });

  test('official keeps non-managed top-level login settings', () => {
    const sb = sandbox();
    try {
      sb.writeSettings(JSON.stringify({ forceLoginMethod: 'console', env: { ANTHROPIC_BASE_URL: 'https://x.example', ANTHROPIC_AUTH_TOKEN: SECRET } }));
      const r = sb.run(['official']);
      assert.equal(r.code, 0, r.all);
      assert.deepEqual(JSON.parse(sb.readSettingsBytes()), { forceLoginMethod: 'console' });
    } finally { sb.cleanup(); }
  });
});

describe('rule 5: no network, no telemetry, no login credential reads', () => {
  const src = fs.readFileSync(CLI, 'utf8');
  test('imports only fs, path, os, crypto and url', () => {
    const imports = [...src.matchAll(/^import .* from '([^']+)';$/gm)].map((m) => m[1]).sort();
    assert.deepEqual(imports, ['node:crypto', 'node:fs', 'node:os', 'node:path', 'node:url']);
    assert.ok(!/\brequire\(|import\(/.test(src), 'no dynamic loading');
  });
  test('no network APIs or credential files referenced', () => {
    for (const bad of [/\bfetch\s*\(/, /XMLHttpRequest/, /WebSocket/, /node:(https?|net|tls|dns|dgram|http2|child_process)/, /\.credentials/, /\.claude\.json/, /keychain/i, /oauth/i]) {
      assert.ok(!bad.test(src), `source must not match ${bad}`);
    }
  });
  test('a profile must have a key, so a custom URL never falls back to a subscription login', () => {
    const sb = sandbox();
    try {
      const r = sb.run(['add', 'nokey', '--base-url', 'https://x.example', '--model', 'm']);
      assert.equal(r.code, 2);
      assert.match(r.stderr, /A key is required/);
    } finally { sb.cleanup(); }
  });
});

describe('rule 6: no secret ever appears in output', () => {
  test('every command, including --json and errors', () => {
    const sb = sandbox();
    try {
      const outputs = [];
      const r1 = sb.run(['add', 'stdin', '--base-url', 'https://a.example/v1', '--model', 'm1', '--key-stdin'], { input: SECRET + '\n' });
      assert.equal(r1.code, 0, r1.all); outputs.push(r1.all);
      outputs.push(addProfile(sb).all);
      for (const args of [['use', 'stdin'], ['status'], ['status', '--json'], ['list'], ['list', '--json'], ['use', 'work'], ['status', '--json'], ['use', 'work'], ['official'], ['restore'], ['use', 'missing'], ['add', 'stdin', '--base-url', 'https://a.example', '--model', 'm', '--key-stdin']]) {
        outputs.push(sb.run(args, { ...withKey, input: SECRET }).all);
      }
      // A key typed as an argument is refused, and not echoed.
      outputs.push(sb.run(['add', 'x', '--base-url', 'https://a.example', '--model', 'm', `--key=${SECRET}`]).all);
      outputs.push(sb.run(['add', 'x', '--base-url', `https://u:${SECRET}@a.example`, '--model', 'm', '--key-env', 'WB_TEST_KEY']).all);
      const all = outputs.join('\n');
      assert.ok(all.length > 500);
      assert.ok(!all.includes(SECRET), 'full key leaked');
      assert.ok(!all.includes(SECRET.slice(4, 20)), 'part of the key leaked');
      assert.ok(all.includes('\u2026tail'), 'masked form shows at most the last 4 characters');
    } finally { sb.cleanup(); }
  });
});

describe('rule 7: coexistence with the wickrunAI desktop app', () => {
  const desktopRoute = {
    theme: 'light',
    env: {
      ANTHROPIC_BASE_URL: 'http://127.0.0.1:47821/anthropic',
      ANTHROPIC_AUTH_TOKEN: 'local-session-token-0123456789abcdef',
      ANTHROPIC_MODEL: 'some-model',
      CLAUDE_CODE_SUBAGENT_MODEL: 'some-model',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'some-model',
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
    },
  };

  test('route takeover: status says so; use/official/restore refuse without --force', () => {
    const sb = sandbox();
    try {
      const text = JSON.stringify(desktopRoute, null, 2) + '\n';
      sb.writeSettings(text);
      const desktopBackup = sb.settings + '.before-wickrun';
      fs.writeFileSync(desktopBackup, 'DESKTOP BACKUP');
      const st = JSON.parse(sb.run(['status', '--json']).stdout);
      assert.equal(st.mode, 'wickrunai-desktop');
      assert.equal(st.desktop.kind, 'route');
      assert.match(sb.run(['status']).stdout, /managed by wickrunAI desktop/);
      assert.ok(!sb.run(['status']).all.includes('local-session-token-0123456789abcdef'));
      addProfile(sb);
      for (const args of [['use', 'work'], ['official']]) {
        const r = sb.run(args, withKey);
        assert.equal(r.code, 1, r.all);
        assert.match(r.stderr, /wickrunAI app/);
        assert.match(r.stderr, /--force/);
        assert.equal(sb.readSettingsBytes().toString('utf8'), text);
      }
      // --force overrides; desktop backup is never touched.
      assert.equal(sb.run(['use', 'work', '--force'], withKey).code, 0);
      assert.equal(JSON.parse(sb.run(['status', '--json']).stdout).mode, 'custom');
      // Simulate the app taking over again, then restore must refuse.
      fs.writeFileSync(sb.settings, text);
      const r = sb.run(['restore']);
      assert.equal(r.code, 1);
      assert.match(r.stderr, /wickrunAI/);
      assert.equal(sb.run(['restore', '--force']).code, 0);
      assert.equal(sb.readSettingsBytes().toString('utf8'), text, 'restored to our original (the desktop shape it had when we first changed it)');
      assert.equal(fs.readFileSync(desktopBackup, 'utf8'), 'DESKTOP BACKUP');
    } finally { sb.cleanup(); }
  });

  test('subscription takeover (forceLoginMethod + desktop backup) is detected', () => {
    const sb = sandbox();
    try {
      sb.writeSettings('{"forceLoginMethod":"claudeai"}');
      assert.equal(JSON.parse(sb.run(['status', '--json']).stdout).mode, 'official');
      fs.writeFileSync(sb.settings + '.before-wickrun', '{}');
      const st = JSON.parse(sb.run(['status', '--json']).stdout);
      assert.equal(st.mode, 'wickrunai-desktop');
      assert.equal(st.desktop.kind, 'subscription');
    } finally { sb.cleanup(); }
  });

  test('a loopback profile of our own is not mistaken for the desktop app', () => {
    const sb = sandbox();
    try {
      addProfile(sb, 'local', { baseUrl: 'http://127.0.0.1:4000' });
      assert.equal(sb.run(['use', 'local'], withKey).code, 0);
      const st = JSON.parse(sb.run(['status', '--json']).stdout);
      assert.equal(st.mode, 'custom');
      assert.equal(st.profile, 'local');
    } finally { sb.cleanup(); }
  });
});

describe('rule 8: profiles file location and key storage', () => {
  test('locations per platform', () => {
    const W = path.win32, X = path.posix;
    const env = { HOME: '/h', USERPROFILE: 'C:\\Users\\u', XDG_CONFIG_HOME: '/x', APPDATA: 'C:\\Users\\u\\AppData\\Roaming' };
    assert.equal(wb.profilesPath(env, 'linux'), X.join('/x', 'wickrun-brain', 'profiles.json'));
    assert.equal(wb.profilesPath({ HOME: '/h' }, 'darwin'), X.join('/h', '.config', 'wickrun-brain', 'profiles.json'));
    assert.equal(wb.profilesPath(env, 'win32'), W.join('C:\\Users\\u\\AppData\\Roaming', 'wickrun-brain', 'profiles.json'));
    assert.equal(wb.profilesPath({ USERPROFILE: 'C:\\Users\\u' }, 'win32'), W.join('C:\\Users\\u', 'AppData', 'Roaming', 'wickrun-brain', 'profiles.json'));
    assert.equal(wb.settingsPath({ HOME: '/h' }, 'linux'), X.join('/h', '.claude', 'settings.json'));
    assert.equal(wb.settingsPath({ HOME: '/h', CLAUDE_CONFIG_DIR: '/cfg' }, 'linux'), X.join('/cfg', 'settings.json'));
    assert.equal(wb.settingsPath({ HOME: '/wrong', USERPROFILE: 'C:\\Users\\u' }, 'win32'), W.join('C:\\Users\\u', '.claude', 'settings.json'));
    assert.throws(() => wb.settingsPath({ HOME: '/h', CLAUDE_CONFIG_DIR: 'relative' }, 'linux'), /absolute/);
    assert.equal(wb.displayPath(W.join('C:\\Users\\U', '.claude', 'settings.json'), { USERPROFILE: 'C:\\Users\\u' }, 'win32'), '~\\.claude\\settings.json');
  });

  test('--key-env stores only the variable name and resolves it at use time', () => {
    const sb = sandbox();
    try {
      addProfile(sb);
      const stored = fs.readFileSync(wb.profilesPath(sb.env), 'utf8');
      assert.ok(!stored.includes(SECRET));
      assert.equal(JSON.parse(stored).profiles.work.keyEnv, 'WB_TEST_KEY');
      const r = sb.run(['use', 'work']);
      assert.equal(r.code, 1);
      assert.match(r.stderr, /WB_TEST_KEY.*not set/);
      assert.equal(sb.readSettingsBytes(), null, 'nothing written when the key is missing');
      assert.equal(sb.run(['use', 'work'], withKey).code, 0);
      const env = JSON.parse(sb.readSettingsBytes()).env;
      assert.equal(env.ANTHROPIC_AUTH_TOKEN, SECRET);
      assert.equal(env.ANTHROPIC_BASE_URL, 'https://llm.example.com');
      for (const k of ['ANTHROPIC_MODEL', 'ANTHROPIC_DEFAULT_OPUS_MODEL', 'ANTHROPIC_DEFAULT_SONNET_MODEL', 'ANTHROPIC_DEFAULT_HAIKU_MODEL', 'ANTHROPIC_DEFAULT_FABLE_MODEL', 'CLAUDE_CODE_SUBAGENT_MODEL']) assert.equal(env[k], 'gw-model-1');
      assert.ok(!('ANTHROPIC_API_KEY' in env));
    } finally { sb.cleanup(); }
  });

  test('--key-stdin stores the key in the profiles file; --auth x-api-key uses ANTHROPIC_API_KEY', () => {
    const sb = sandbox();
    try {
      const r = sb.run(['add', 's', '--base-url', 'https://a.example', '--model', 'm', '--key-stdin', '--auth', 'x-api-key'], { input: `  ${SECRET}\r\n` });
      assert.equal(r.code, 0, r.all);
      assert.match(r.stdout, /stored in the profiles file/);
      assert.equal(JSON.parse(fs.readFileSync(wb.profilesPath(sb.env), 'utf8')).profiles.s.key, SECRET);
      sb.writeSettings(JSON.stringify({ env: { ANTHROPIC_AUTH_TOKEN: 'old-token-value' } }));
      assert.equal(sb.run(['use', 's']).code, 0);
      const env = JSON.parse(sb.readSettingsBytes()).env;
      assert.equal(env.ANTHROPIC_API_KEY, SECRET);
      assert.ok(!('ANTHROPIC_AUTH_TOKEN' in env), 'stale credential removed');
    } finally { sb.cleanup(); }
  });

  test('settings under HOME (no CLAUDE_CONFIG_DIR), paths with spaces', () => {
    const sb = sandbox({ configDir: false });
    try {
      assert.ok(sb.settings.includes(' '));
      addProfile(sb);
      assert.equal(sb.run(['use', 'work'], withKey).code, 0);
      assert.ok(fs.existsSync(path.join(sb.home, '.claude', 'settings.json')));
      assert.match(sb.run(['status']).stdout, /~[\\/]\.claude[\\/]settings\.json/);
    } finally { sb.cleanup(); }
  });
});
