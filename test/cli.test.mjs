import { test, describe } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { sandbox, addProfile, withKey, SECRET } from './helpers.mjs';
import * as wb from '../scripts/wickrun-brain.mjs';

describe('argument validation', () => {
  const bad = [
    ['ftp URL', ['--base-url', 'ftp://a.example', '--model', 'm'], /https:\/\/ or http:\/\//],
    ['not a URL', ['--base-url', 'llm.example.com', '--model', 'm'], /not a valid URL/],
    ['credentials in URL', ['--base-url', 'https://user:pw@a.example', '--model', 'm'], /user name or password/],
    ['query string', ['--base-url', 'https://a.example/?k=v', '--model', 'm'], /query/],
    ['bare ?', ['--base-url', 'https://a.example/?', '--model', 'm'], /query/],
    ['fragment', ['--base-url', 'https://a.example/#x', '--model', 'm'], /fragment/],
    ['model with space', ['--base-url', 'https://a.example', '--model', 'a b'], /--model/],
    ['model too long', ['--base-url', 'https://a.example', '--model', 'x'.repeat(201)], /--model/],
    ['model empty', ['--base-url', 'https://a.example', '--model='], /--model/],
    ['missing model', ['--base-url', 'https://a.example'], /--model/],
    ['missing base url', ['--model', 'm'], /--base-url is required/],
    ['bad auth', ['--base-url', 'https://a.example', '--model', 'm', '--auth', 'basic'], /--auth/],
  ];
  for (const [label, args, re] of bad) {
    test(label, () => {
      const sb = sandbox();
      try {
        const r = sb.run(['add', 'p', ...args, '--key-env', 'WB_TEST_KEY']);
        assert.equal(r.code, 2, r.all);
        assert.match(r.stderr, re);
        assert.ok(!fs.existsSync(wb.profilesPath(sb.env)));
      } finally { sb.cleanup(); }
    });
  }

  test('accepted model ids and URLs', () => {
    const sb = sandbox();
    try {
      for (const [i, m] of ['claude-sonnet-4-6', 'org/model.v2:latest@rev+1', 'a_b', 'x'.repeat(200)].entries()) {
        const r = sb.run(['add', `p${i}`, '--base-url', i % 2 ? 'http://127.0.0.1:8080/api' : 'https://gw.example.com/anthropic/', '--model', m, '--key-env', 'K']);
        assert.equal(r.code, 0, r.all);
      }
    } finally { sb.cleanup(); }
  });

  test('keys are refused as arguments', () => {
    const sb = sandbox();
    try {
      for (const flag of ['--key', '--api-key', '--token', '--auth-token']) {
        const r = sb.run(['add', 'p', '--base-url', 'https://a.example', '--model', 'm', flag, SECRET]);
        assert.equal(r.code, 2);
        assert.match(r.stderr, /never accepted on the command line/);
        assert.ok(!r.all.includes(SECRET));
      }
    } finally { sb.cleanup(); }
  });

  test('key source rules', () => {
    const sb = sandbox();
    try {
      const base = ['add', 'p', '--base-url', 'https://a.example', '--model', 'm'];
      assert.match(sb.run([...base, '--key-env', 'K', '--key-stdin']).stderr, /not both/);
      assert.match(sb.run([...base, '--key-env', '1BAD']).stderr, /NAME of an environment variable/);
      assert.match(sb.run([...base, '--key-env', 'HAS-DASH']).stderr, /NAME of an environment variable/);
      const empty = sb.run([...base, '--key-stdin'], { input: '' });
      assert.equal(empty.code, 1); assert.match(empty.stderr, /empty/);
      const spaced = sb.run([...base, '--key-stdin'], { input: 'two words' });
      assert.equal(spaced.code, 1); assert.match(spaced.stderr, /spaces/);
    } finally { sb.cleanup(); }
  });

  test('profile names', () => {
    const sb = sandbox();
    try {
      for (const n of ['-x', '.hidden', 'a b', 'a/b', 'x'.repeat(65), '']) {
        const r = sb.run(['add', n, '--base-url', 'https://a.example', '--model', 'm', '--key-env', 'K']);
        assert.equal(r.code, 2, `${JSON.stringify(n)}: ${r.all}`);
      }
      assert.equal(sb.run(['add', 'ok.name_1-2', '--base-url', 'https://a.example', '--model', 'm', '--key-env', 'K']).code, 0);
      const dup = sb.run(['add', 'ok.name_1-2', '--base-url', 'https://a.example', '--model', 'm', '--key-env', 'K']);
      assert.equal(dup.code, 1); assert.match(dup.stderr, /--force/);
      assert.equal(sb.run(['add', 'ok.name_1-2', '--base-url', 'https://b.example', '--model', 'm', '--key-env', 'K', '--force']).code, 0);
    } finally { sb.cleanup(); }
  });

  test('plain http to a remote host warns; loopback does not', () => {
    const sb = sandbox();
    try {
      assert.match(sb.run(['add', 'r', '--base-url', 'http://gw.example', '--model', 'm', '--key-env', 'K'], { extraEnv: { K: 'x' } }).stderr, /unencrypted/);
      assert.doesNotMatch(sb.run(['add', 'l', '--base-url', 'http://localhost:9', '--model', 'm', '--key-env', 'K'], { extraEnv: { K: 'x' } }).stderr, /unencrypted/);
    } finally { sb.cleanup(); }
  });

  test('usage errors exit 2', () => {
    const sb = sandbox();
    try {
      assert.equal(sb.run(['frobnicate']).code, 2);
      assert.equal(sb.run(['status', '--bogus']).code, 2);
      assert.equal(sb.run(['status', 'extra']).code, 2);
      assert.equal(sb.run(['use']).code, 2);
      assert.equal(sb.run(['use', 'a', 'b']).code, 2);
      assert.equal(sb.run(['use', 'a', '--json']).code, 2);
      assert.equal(sb.run(['status', '--json=1']).code, 2);
      assert.equal(sb.run([]).code, 2);
    } finally { sb.cleanup(); }
  });

  test('--help and --version', () => {
    const sb = sandbox();
    try {
      const h = sb.run(['--help']);
      assert.equal(h.code, 0);
      for (const c of ['status', 'list', 'add', 'use', 'official', 'restore', 'remove']) assert.match(h.stdout, new RegExp(`wickrun-brain ${c}`));
      const v = sb.run(['--version']);
      assert.equal(v.code, 0);
      assert.equal(v.stdout.trim(), wb.VERSION);
      const pkg = JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url)));
      const plugin = JSON.parse(fs.readFileSync(new URL('../.claude-plugin/plugin.json', import.meta.url)));
      assert.equal(pkg.version, wb.VERSION);
      assert.equal(plugin.version, wb.VERSION);
    } finally { sb.cleanup(); }
  });
});

describe('--json shapes', () => {
  test('status --json', () => {
    const sb = sandbox();
    try {
      let st = JSON.parse(sb.run(['status', '--json']).stdout);
      assert.deepEqual(Object.keys(st).sort(), ['backup', 'baseUrlHost', 'credential', 'desktop', 'keysSet', 'mode', 'modeLabel', 'model', 'overridden', 'profile', 'settingsExists', 'settingsFile', 'tool', 'version'].sort());
      assert.equal(st.mode, 'official');
      assert.equal(st.modeLabel, 'official login (no override)');
      assert.equal(st.overridden, false);
      assert.deepEqual(st.keysSet, []);
      assert.equal(st.credential, null);
      assert.equal(st.settingsExists, false);
      assert.deepEqual(st.backup, { exists: false, path: null });
      assert.deepEqual(st.desktop, { detected: false, kind: null });
      addProfile(sb, 'work', { baseUrl: 'https://gw.example.com:8443/anthropic' });
      sb.run(['use', 'work'], withKey);
      st = JSON.parse(sb.run(['status', '--json']).stdout);
      assert.equal(st.mode, 'custom');
      assert.equal(st.modeLabel, 'custom endpoint');
      assert.equal(st.baseUrlHost, 'gw.example.com:8443');
      assert.equal(st.model, 'gw-model-1');
      assert.equal(st.profile, 'work');
      assert.deepEqual(st.credential, { variable: 'ANTHROPIC_AUTH_TOKEN', masked: '…tail' });
      assert.equal(st.keysSet.length, 8);
      assert.equal(st.backup.exists, true);
    } finally { sb.cleanup(); }
  });

  test('list --json and remove', () => {
    const sb = sandbox();
    try {
      assert.deepEqual(JSON.parse(sb.run(['list', '--json']).stdout).profiles, []);
      assert.match(sb.run(['list']).stdout, /No saved profiles/);
      addProfile(sb, 'b');
      sb.run(['add', 'a', '--base-url', 'https://a.example', '--model', 'm', '--key-stdin'], { input: SECRET });
      sb.run(['use', 'b'], withKey);
      const l = JSON.parse(sb.run(['list', '--json']).stdout);
      assert.deepEqual(l.profiles, [
        { name: 'a', baseUrlHost: 'a.example', model: 'm', auth: 'bearer', keySource: 'stored', active: false },
        { name: 'b', baseUrlHost: 'llm.example.com', model: 'gw-model-1', auth: 'bearer', keySource: 'env', keyEnv: 'WB_TEST_KEY', active: true },
      ]);
      assert.match(sb.run(['list']).stdout, /\* b/);
      const before = sb.readSettingsBytes();
      assert.equal(sb.run(['remove', 'b']).code, 0);
      assert.deepEqual(sb.readSettingsBytes(), before, 'remove does not touch settings.json');
      assert.equal(sb.run(['remove', 'b']).code, 1);
      assert.equal(JSON.parse(sb.run(['list', '--json']).stdout).profiles.length, 1);
    } finally { sb.cleanup(); }
  });
});

describe('behaviour', () => {
  test('use twice is a no-op the second time', () => {
    const sb = sandbox();
    try {
      addProfile(sb);
      sb.run(['use', 'work'], withKey);
      const r = sb.run(['use', 'work'], withKey);
      assert.equal(r.code, 0);
      assert.match(r.stdout, /already active/);
    } finally { sb.cleanup(); }
  });

  test('use of an unknown profile fails without writing', () => {
    const sb = sandbox();
    try {
      const r = sb.run(['use', 'nope']);
      assert.equal(r.code, 1);
      assert.match(r.stderr, /No profile named "nope"/);
      assert.equal(sb.readSettingsBytes(), null);
    } finally { sb.cleanup(); }
  });

  test('mode changes print the new-session hint', () => {
    const sb = sandbox();
    try {
      addProfile(sb);
      for (const args of [['use', 'work'], ['official'], ['restore']]) assert.match(sb.run(args, withKey).stdout, /Start a new Claude Code session/);
    } finally { sb.cleanup(); }
  });
});

describe('unit', () => {
  test('maskSecret', () => {
    assert.equal(wb.maskSecret(SECRET), '…tail');
    assert.equal(wb.maskSecret('short-key'), 'set');
    assert.equal(wb.maskSecret(''), null);
    assert.equal(wb.maskSecret(undefined), null);
  });
  test('parseArgs', () => {
    assert.deepEqual(wb.parseArgs(['add', 'n', '--model=a', '--base-url', 'u', '--key-stdin']), { pos: ['add', 'n'], opts: { model: 'a', 'base-url': 'u', 'key-stdin': true } });
    assert.throws(() => wb.parseArgs(['--model']), /needs a value/);
    assert.throws(() => wb.parseArgs(['--model', '--json']), /needs a value/);
  });
  test('displayPath', () => {
    assert.equal(wb.displayPath('/h/u/.claude/settings.json', { HOME: '/h/u' }, 'linux'), '~/.claude/settings.json');
    assert.equal(wb.displayPath('/other/settings.json', { HOME: '/h/u' }, 'linux'), '/other/settings.json');
  });
});
