#!/usr/bin/env node
// Builds the README demo animation (pure SVG + CSS keyframes + SMIL, no scripts,
// no external resources). The terminal text is captured by running the real CLI
// in a throwaway home folder, so the animation shows exactly what the tool prints.
//
//   node scripts/build-demo.mjs                 -> docs/demo/demo-{en,zh}-{light,dark}.svg
//   node scripts/build-demo.mjs --frame 9.5 [--lang en|zh] [--theme light|dark] [--out file.svg]
//                                               -> one static (non-animated) frame
//
// Zero dependencies. Deterministic: same CLI version -> same bytes.
// SPDX-License-Identifier: MIT

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const CLI = path.join(ROOT, 'scripts', 'wickrun-brain.mjs');
const OUT_DIR = path.join(ROOT, 'docs', 'demo');

const DEMO_KEY = 'sk-demo-0000000000000000000000000000-7f3a'; // fake, shown masked as ...7f3a
const PROFILE = 'work-gateway';
const BASE_URL = 'https://llm.example.com';
const MODEL = 'claude-sonnet-4-6';

/* ---------------------------------------------------- capture real output */

function capture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wb-demo-'));
  try {
    const home = path.join(root, 'home');
    fs.mkdirSync(path.join(home, '.claude'), { recursive: true });
    fs.writeFileSync(path.join(home, '.claude', 'settings.json'), '{\n  "theme": "dark"\n}\n');
    const env = { PATH: process.env.PATH || '', HOME: home, USERPROFILE: home, XDG_CONFIG_HOME: path.join(root, 'xdg'), APPDATA: path.join(root, 'appdata'), DEMO_GATEWAY_KEY: DEMO_KEY };
    for (const k of ['SystemRoot', 'TEMP', 'TMP']) if (process.env[k]) env[k] = process.env[k];
    const run = (args) => {
      const r = spawnSync(process.execPath, [CLI, ...args], { env, encoding: 'utf8' });
      if (r.status !== 0) throw new Error(`demo command failed: ${args.join(' ')}\n${r.stderr}`);
      if ((r.stdout + r.stderr).includes(DEMO_KEY)) throw new Error('demo output contains the key');
      return r.stdout.replace(/\r\n/g, '\n').replace(/\n$/, '').split('\n');
    };
    run(['add', PROFILE, '--base-url', BASE_URL, '--model', MODEL, '--key-env', 'DEMO_GATEWAY_KEY']);
    return [
      { cmd: 'wickrun-brain status', out: run(['status']) },
      { cmd: `wickrun-brain use ${PROFILE}`, out: run(['use', PROFILE]) },
      { cmd: 'wickrun-brain status', out: run(['status']) },
      { cmd: 'wickrun-brain restore', out: run(['restore']) },
    ];
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
}

/* ------------------------------------------------------------- layout */

const TEXT = {
  en: {
    title: 'terminal',
    caption: 'Illustration of real command output',
    comments: ['# What is Claude Code using right now?', '# Switch to a saved profile', '# Check again: key is masked', '# Undo everything'],
  },
  zh: {
    title: '终端',
    caption: '真实命令输出的示意',
    comments: ['# Claude Code 现在用的是哪个大脑？', '# 切换到已保存的配置', '# 再看一次：密钥已打码', '# 一键还原'],
  },
};
const THEMES = {
  light: { page: '#ffffff', frame: '#d0d7de', bar: '#eaeef2', term: '#f6f8fa', text: '#1f2328', dim: '#6e7781', prompt: '#0969da', accent: '#1a7f37', caption: '#57606a' },
  dark: { page: '#0d1117', frame: '#30363d', bar: '#21262d', term: '#161b22', text: '#e6edf3', dim: '#8b949e', prompt: '#58a6ff', accent: '#3fb950', caption: '#8b949e' },
};

const W = 860;
const FONT = 13;
const LH = 19;
const CHAR = 7.85; // approx. advance of a 13px monospace glyph, used for the typing clip
const PAD_X = 22;
const BAR_H = 30;
const TOP = 54; // baseline of the first terminal line
const LOOP = 18; // seconds
const SCENES = [0, 4.4, 8.6, 13.2]; // scene start times
const SCENE_END = [4.3, 8.5, 13.1, 17.4];
const TYPE_START = 0.35; // after scene start
const CPS = 26; // typing speed, characters per second
const OUT_DELAY = 0.35; // after typing finishes
const LINE_STAGGER = 0.05;

function esc(s) { return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }
const fmt = (n) => Number(n.toFixed(3)).toString();
const pct = (t) => fmt((t / LOOP) * 100) + '%';

function buildTimeline(scenes, lang) {
  const items = []; // { kind, text, y, appear, end, scene, typeStart?, typeEnd? }
  let maxLines = 0;
  scenes.forEach((sc, i) => {
    const start = SCENES[i];
    const end = SCENE_END[i];
    let y = TOP;
    items.push({ kind: 'comment', text: TEXT[lang].comments[i], y, appear: start + 0.05, end, scene: i });
    y += LH;
    const typeStart = start + TYPE_START;
    const typeEnd = typeStart + sc.cmd.length / CPS;
    items.push({ kind: 'cmd', text: sc.cmd, y, appear: typeStart, end, typeStart, typeEnd, scene: i });
    y += LH;
    sc.out.forEach((line, j) => {
      items.push({ kind: 'out', text: line, y, appear: typeEnd + OUT_DELAY + j * LINE_STAGGER, end, scene: i });
      y += LH;
    });
    maxLines = Math.max(maxLines, 2 + sc.out.length);
  });
  return { items, maxLines };
}

function lineMarkup(it, c) {
  if (it.kind === 'comment') return `<tspan fill="${c.dim}">${esc(it.text)}</tspan>`;
  if (it.kind === 'cmd') return `<tspan fill="${c.prompt}">$ </tspan><tspan fill="${c.text}">${esc(it.text)}</tspan>`;
  const m = /^( {2}mode {12})(.*)$/.exec(it.text);
  if (m) return `<tspan fill="${c.dim}">${esc(m[1])}</tspan><tspan fill="${c.accent}">${esc(m[2])}</tspan>`;
  const kv = /^( {2}[a-zA-Z ]+? {2,})(.*)$/.exec(it.text);
  if (kv) return `<tspan fill="${c.dim}">${esc(kv[1])}</tspan><tspan fill="${c.text}">${esc(kv[2])}</tspan>`;
  if (/byte-identical/.test(it.text)) return `<tspan fill="${c.accent}">${esc(it.text)}</tspan>`;
  return `<tspan fill="${c.text}">${esc(it.text)}</tspan>`;
}

function render(scenes, { lang, theme, frame = null }) {
  const c = THEMES[theme];
  const { items, maxLines } = buildTimeline(scenes, lang);
  const termH = TOP - BAR_H + maxLines * LH;
  const H = BAR_H + termH + 34;
  const font = `ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', 'Noto Sans Mono CJK SC', monospace`;
  const parts = [];
  parts.push(`<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-labelledby="t d" xml:space="preserve">`);
  parts.push(`<title id="t">wickrun-brain demo</title>`);
  parts.push(`<desc id="d">${esc(scenes.map((s) => '$ ' + s.cmd + '\n' + s.out.join('\n')).join('\n\n'))}</desc>`);
  const css = [`text{font-family:${font};font-size:${FONT}px;white-space:pre}`, `.cap{font-family:system-ui,-apple-system,'Segoe UI',Roboto,'Noto Sans CJK SC',sans-serif;font-size:12px}`];
  if (frame === null) {
    items.forEach((it, i) => {
      css.push(`@keyframes k${i}{0%,${pct(it.appear)}{opacity:0}${pct(it.appear + 0.001)},${pct(it.end)}{opacity:1}${pct(it.end + 0.001)},100%{opacity:0}}`);
      css.push(`.l${i}{opacity:0;animation:k${i} ${LOOP}s linear infinite}`);
    });
    css.push('@media (prefers-reduced-motion:reduce){.anim text{animation:none!important;opacity:0!important}.anim text.s2{opacity:1!important}}');
  }
  parts.push(`<style>${css.join('')}</style>`);
  // window
  parts.push(`<rect x="0.5" y="0.5" width="${W - 1}" height="${BAR_H + termH}" rx="9" fill="${c.term}" stroke="${c.frame}"/>`);
  parts.push(`<path d="M0.5 ${BAR_H}V9.5a9 9 0 0 1 9-9H${W - 9.5}a9 9 0 0 1 9 9V${BAR_H}Z" fill="${c.bar}" stroke="${c.frame}"/>`);
  ['#ff5f57', '#febc2e', '#28c840'].forEach((col, i) => parts.push(`<circle cx="${20 + i * 18}" cy="${BAR_H / 2}" r="5.5" fill="${col}"/>`));
  parts.push(`<text x="${W / 2}" y="${BAR_H / 2 + 4}" text-anchor="middle" fill="${c.dim}" class="cap">${esc(TEXT[lang].title)}</text>`);
  parts.push(`<g class="anim">`);
  items.forEach((it, i) => {
    if (frame !== null && !(frame >= it.appear && frame < it.end)) return;
    const cls = frame === null ? ` class="l${i} s${it.scene}"` : '';
    if (it.kind === 'cmd') {
      const full = 2 * CHAR + it.text.length * CHAR + 4;
      const clipId = `c${i}`;
      if (frame === null) {
        const kt = [0, it.typeStart / LOOP, it.typeEnd / LOOP, it.end / LOOP, 1].map(fmt).join(';');
        parts.push(`<clipPath id="${clipId}"><rect x="${PAD_X}" y="${it.y - FONT - 2}" height="${LH + 2}" width="0"><animate attributeName="width" dur="${LOOP}s" repeatCount="indefinite" calcMode="linear" keyTimes="${kt}" values="${fmt(2 * CHAR)};${fmt(2 * CHAR)};${fmt(full)};${fmt(full)};${fmt(2 * CHAR)}"/></rect></clipPath>`);
      } else {
        const p = Math.max(0, Math.min(1, (frame - it.typeStart) / (it.typeEnd - it.typeStart)));
        parts.push(`<clipPath id="${clipId}"><rect x="${PAD_X}" y="${it.y - FONT - 2}" height="${LH + 2}" width="${fmt(2 * CHAR + p * (full - 2 * CHAR))}"/></clipPath>`);
      }
      parts.push(`<text x="${PAD_X}" y="${it.y}"${cls} clip-path="url(#${clipId})">${lineMarkup(it, c)}</text>`);
    } else {
      parts.push(`<text x="${PAD_X}" y="${it.y}"${cls}>${lineMarkup(it, c)}</text>`);
    }
  });
  parts.push(`</g>`);
  parts.push(`<text x="${W / 2}" y="${H - 12}" text-anchor="middle" fill="${c.caption}" class="cap">${esc(TEXT[lang].caption)}</text>`);
  parts.push(`</svg>`);
  return parts.join('\n') + '\n';
}

/* ---------------------------------------------------------------- main */

function arg(name, def) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : def;
}

const scenes = capture();
if (process.argv.includes('--frame')) {
  const t = Number(arg('frame'));
  if (!Number.isFinite(t) || t < 0 || t > LOOP) throw new Error(`--frame must be between 0 and ${LOOP}`);
  const svg = render(scenes, { lang: arg('lang', 'en'), theme: arg('theme', 'light'), frame: t });
  const out = arg('out');
  if (out) fs.writeFileSync(out, svg); else process.stdout.write(svg);
} else {
  fs.mkdirSync(OUT_DIR, { recursive: true });
  for (const lang of ['en', 'zh']) for (const theme of ['light', 'dark']) {
    const file = path.join(OUT_DIR, `demo-${lang}-${theme}.svg`);
    const svg = render(scenes, { lang, theme });
    if (Buffer.byteLength(svg) > 100 * 1024) throw new Error(`${file} is over 100 KB`);
    fs.writeFileSync(file, svg);
    console.log(`${path.relative(ROOT, file)}  ${Buffer.byteLength(svg)} bytes`);
  }
}
