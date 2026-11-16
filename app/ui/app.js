// wickrun-brain front end. Plain JS, no build step. SPDX-License-Identifier: MIT
(() => {
  const T = window.__TAURI__;
  const invoke = T ? T.core.invoke : window.WB_MOCK.invoke;
  const I = window.WB_I18N;
  const LANGS = ['zh-Hans', 'zh-Hant', 'en'];

  function systemLang() {
    const l = (navigator.languages && navigator.languages[0]) || navigator.language || 'en';
    const s = l.toLowerCase();
    if (s.startsWith('zh')) return /hant|tw|hk|mo/.test(s) ? 'zh-Hant' : 'zh-Hans';
    return 'en';
  }
  const params = new URLSearchParams(location.search);
  let lang = params.get('lang') || localStorage.getItem('wb-lang') || systemLang();
  if (!I[lang]) lang = 'en';

  const t = (k, vars) => {
    let s = (I[lang] && I[lang][k]) ?? I.en[k] ?? k;
    if (vars) for (const [a, b] of Object.entries(vars)) s = s.replaceAll(`{${a}}`, b);
    return s;
  };
  const colon = () => (lang === 'en' ? ': ' : '：');
  const $ = (id) => document.getElementById(id);
  const el = (tag, attrs = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v === undefined || v === null || v === false) continue;
      if (k === 'class') e.className = v;
      else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? '' : v);
    }
    for (const c of kids.flat()) if (c !== null && c !== undefined && c !== false) e.append(c.nodeType ? c : String(c));
    return e;
  };

  let view = null;
  let loading = true;
  const ui = {}; // per-source UI state: model, custom, endpoint, open, result, models

  let toastTimer;
  function toast(msg, bad) {
    const n = $('toast');
    n.textContent = msg;
    n.className = 'toast show' + (bad ? ' bad' : '');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (n.className = 'toast'), bad ? 6000 : 4200);
  }
  function errText(e) {
    const code = e && e.code ? e.code : 'other';
    const k = 'e_' + code;
    return I.en[k] ? t(k) : t('e_other');
  }

  function originText(o) {
    if (!o) return '';
    if (o.startsWith('env:')) return t('o_env');
    if (o.startsWith('url:')) return t('o_url', { url: o.slice(4) });
    if (o.startsWith('file:')) {
      let f = o.slice(5).split('#')[0];
      if (f.endsWith('cc-switch.db')) return t('o_ccswitch');
      return t('o_file', { file: f });
    }
    if (o === 'settings') return t('o_settings');
    return o;
  }

  /* ------------------------------------------------------------- current */
  function renderCurrent() {
    const c = view.current;
    const box = $('current');
    box.replaceChildren();
    box.append(el('p', { class: 'label' }, t('nowUsing')));
    const label = c.label || '';
    let main;
    let hint = null;
    switch (c.kind) {
      case 'official': main = t('cur_official'); break;
      case 'proxy': main = t('cur_proxy', { label }); break;
      case 'proxy_down': main = t('cur_proxy_down', { label }); hint = el('p', { class: 'hint bad' }, view.proxy_error ? t('proxy_' + view.proxy_error) + '. ' : '', t('cur_proxy_down_hint')); break;
      case 'wickrunai': main = t('cur_wickrunai'); hint = el('p', { class: 'hint warn' }, t('cur_wickrunai_hint')); break;
      case 'broken': main = t('cur_broken'); hint = el('p', { class: 'hint bad' }, t('cur_broken_hint')); break;
      default: main = t('cur_custom', { label });
    }
    const showModel = c.model && c.kind !== 'official' && c.kind !== 'wickrunai';
    box.append(el('p', { class: 'brain' }, main, showModel ? el('span', { class: 'model' }, `${t('model')}${colon()}${c.model}`) : null));
    if (hint) box.append(hint);
    const actions = el('div', { class: 'actions' });
    if (c.kind === 'wickrunai') {
      actions.append(el('button', { class: 'primary', onclick: () => invoke('open_wickrunai').catch((e) => toast(errText(e), true)) }, t('openWickrun')));
    }
    actions.append(el('button', { class: c.kind === 'proxy_down' || c.kind === 'broken' ? 'primary' : '', disabled: !c.backup_exists || c.kind === 'wickrunai', title: t('restoreHint'), onclick: doRestore }, t('restore')));
    if (c.kind === 'custom' || c.kind === 'proxy' || c.kind === 'proxy_down') {
      actions.append(el('button', { class: 'ghost', onclick: doOfficial }, t('official')));
    }
    const open = ui.__current;
    actions.append(el('button', { class: 'link', 'aria-expanded': open ? 'true' : 'false', onclick: () => { ui.__current = !open; renderCurrent(); } }, open ? t('hideDetails') : t('details')));
    box.append(actions);
    if (open) {
      const rows = [[t('d_settings'), c.settings_file]];
      if (c.base_url) rows.push([t('d_address'), c.base_url]);
      if (c.credential_var) rows.push([t('d_keyvar'), `${c.credential_var} ${c.credential_masked || ''}`]);
      rows.push([t('d_translator'), view.proxy_running ? t('d_translator_on', { port: view.proxy_port }) : t('d_translator_off')]);
      box.append(el('dl', { class: 'details' }, rows.flatMap(([a, b]) => [el('dt', {}, a), el('dd', {}, b)])));
    }
  }

  /* -------------------------------------------------------------- sources */
  function isActive(s) {
    const c = view.current;
    if (!['custom', 'proxy', 'proxy_down'].includes(c.kind)) return false;
    return view.active_id ? view.active_id === s.id : (c.kind === 'custom' && s.route === 'direct' && (c.base_url || '').replace(/\/$/, '') === s.base_url.replace(/\/$/, '') && c.label === s.name);
  }

  function chosenModel(s) {
    const u = ui[s.id] || {};
    if (u.custom) return (u.customValue || '').trim();
    return u.model || s.default_model || (u.models && u.models[0]) || '';
  }

  async function loadModels(s) {
    const u = (ui[s.id] = ui[s.id] || {});
    if (u.models || u.loadingModels) return;
    u.loadingModels = true;
    renderGroups();
    try { u.models = await invoke('list_models', { id: s.id }); } catch { u.models = s.models; }
    u.loadingModels = false;
    if (!u.model && u.models.length && !s.default_model) u.model = u.models[0];
    renderGroups();
  }

  function sourceItem(s) {
    const u = (ui[s.id] = ui[s.id] || {});
    const active = isActive(s);
    const usable = s.status === 'reachable' && s.route !== 'none';
    const item = el('div', { class: 'item' + (active ? ' active' : '') + (s.status === 'not_compatible' ? ' dim' : '') });
    item.append(el('div', { class: 'row' },
      el('span', { class: 'name' }, s.name),
      active ? el('span', { class: 'pill using' }, t('inUse')) : el('span', { class: 'pill ' + s.status }, t('s_' + s.status)),
    ));
    item.append(el('p', { class: 'origin' }, originText(s.origin)));
    const note = s.note ? t('note_' + s.note) : '';
    if (note) item.append(el('p', { class: 'note' }, note));

    if (s.group === 'wickrunai') {
      item.append(el('div', { class: 'controls' }, el('button', { class: 'primary', onclick: () => invoke('open_wickrunai').catch((e) => toast(errText(e), true)) }, t('openWickrun'))));
      return item;
    }

    if (s.status === 'needs_key') {
      const input = el('input', { type: 'password', class: 'grow', placeholder: t('pasteKey'), 'aria-label': t('pasteKey'), autocomplete: 'off', spellcheck: 'false' });
      item.append(el('div', { class: 'controls' }, input, el('button', {
        onclick: async () => {
          const k = input.value.trim();
          if (!k) return;
          try { view = await invoke('set_key', { id: s.id, key: k }); input.value = ''; toast(t('keySaved')); render(); } catch (e) { toast(errText(e), true); }
        },
      }, t('saveKey'))));
    }

    if (usable) {
      const models = u.models || s.models || [];
      const controls = el('div', { class: 'controls' });
      if (s.endpoints && s.endpoints.length) {
        const sel = el('select', { 'aria-label': t('endpoint'), onchange: (e) => { u.endpoint = e.target.value; } },
          s.endpoints.map(([lab, url]) => el('option', { value: url, selected: (u.endpoint || s.base_url) === url }, `${t('endpoint')}${colon()}${t('ep_' + lab)}`)));
        controls.append(sel);
      }
      if (u.custom || (!models.length && !u.loadingModels)) {
        u.custom = true;
        controls.append(el('input', { type: 'text', class: 'grow', placeholder: t('typeModel'), 'aria-label': t('model'), value: u.customValue || '', spellcheck: 'false', oninput: (e) => { u.customValue = e.target.value; } }));
      } else {
        const sel = el('select', { class: 'grow', 'aria-label': t('model'), onfocus: () => loadModels(s), onchange: (e) => {
          if (e.target.value === '__other') { u.custom = true; renderGroups(); } else u.model = e.target.value;
        } },
          u.loadingModels ? el('option', {}, t('loadingModels')) : null,
          models.map((m) => el('option', { value: m, selected: chosenModel(s) === m }, m)),
          el('option', { value: '__other' }, t('otherModel')));
        controls.append(sel);
      }
      controls.append(el('button', { disabled: u.busy, onclick: () => doTest(s) }, u.busy === 'test' ? t('testing') : t('test')));
      controls.append(el('button', { class: 'primary', disabled: u.busy, onclick: () => doUse(s) }, t('use')));
      item.append(controls);
      if (!u.models && s.list_models !== false && (s.group === 'keys')) loadModels(s);
    }
    if (u.result) item.append(el('p', { class: 'result ' + (u.result.ok ? 'ok' : 'bad') }, u.result.text));

    const open = u.open;
    item.append(el('div', {}, el('button', { class: 'link', 'aria-expanded': open ? 'true' : 'false', onclick: () => { u.open = !open; renderGroups(); } }, open ? t('hideDetails') : t('details'))));
    if (open) {
      const rows = [[t('d_where'), s.origin.replace(/^(env|file|url):/, '')]];
      if (s.base_url) rows.push([t('d_address'), s.base_url]);
      if (s.key_masked) rows.push([t('d_key'), s.key_masked]);
      if (s.route !== 'none') rows.push([t('d_route'), s.route === 'direct' ? t('d_route_direct') : t('d_route_translate')]);
      const dl = el('dl', { class: 'details' }, rows.flatMap(([a, b]) => [el('dt', {}, a), el('dd', {}, b)]));
      if (s.docs) dl.append(el('dt', {}, t('d_docs')), el('dd', {}, el('a', { href: '#', onclick: (e) => { e.preventDefault(); invoke('open_link', { url: s.docs }); } }, s.docs)));
      item.append(dl);
    }
    return item;
  }

  const GROUPS = ['local', 'keys', 'ccswitch', 'gateway', 'wickrunai'];
  function renderGroups() {
    const g = $('groups');
    g.replaceChildren();
    if (loading && !view.sources.length) { g.append(el('p', { class: 'spin' }, t('searching'))); return; }
    for (const name of GROUPS) {
      const items = view.sources.filter((s) => s.group === name);
      if (!items.length && name === 'wickrunai') continue;
      g.append(el('h2', {}, t('g_' + name)));
      if (!items.length) g.append(el('p', { class: 'empty' }, t('g_' + name + '_empty')));
      else g.append(el('div', { class: 'list' }, items.map(sourceItem)));
    }
  }

  function renderStatic() {
    document.documentElement.lang = lang;
    $('title').textContent = t('appTitle');
    $('notice').textContent = t('notice');
    $('refresh').textContent = t('refresh');
    $('autostartLabel').textContent = t('autostart');
    $('autostartHint').textContent = t('autostartHint');
    $('footerWickrun').textContent = t('footerWickrun');
    $('footerWickrunLink').textContent = t('footerWickrunLink');
    $('version').textContent = `wickrun-brain ${view ? view.version : ''} · MIT`;
    const langs = $('langs');
    langs.replaceChildren(...LANGS.map((l) => el('button', { 'aria-pressed': l === lang ? 'true' : 'false', lang: l, onclick: () => { lang = l; localStorage.setItem('wb-lang', l); render(); } }, I[l].langName)));
    langs.setAttribute('aria-label', t('lang'));
  }

  function render() {
    renderStatic();
    if (!view) return;
    renderCurrent();
    renderGroups();
  }

  /* -------------------------------------------------------------- actions */
  async function doTest(s) {
    const u = ui[s.id];
    const model = chosenModel(s);
    u.busy = 'test'; u.result = null; renderGroups();
    try {
      const r = await invoke('test_source', { id: s.id, model, endpoint: u.endpoint || null });
      u.result = r.ok ? { ok: true, text: t('test_ok', { ms: r.ms }) } : { ok: false, text: t('test_' + (r.code || 'other')) + (r.status ? ` (HTTP ${r.status})` : '') };
    } catch (e) { u.result = { ok: false, text: errText(e) }; }
    u.busy = false; renderGroups();
  }

  async function doUse(s) {
    const u = ui[s.id];
    const model = chosenModel(s);
    if (!model) { toast(t('e_bad_model'), true); return; }
    u.busy = 'use'; renderGroups();
    try {
      view = await invoke('use_source', { id: s.id, model, endpoint: u.endpoint || null });
      toast(s.route === 'translate' ? t('switchedProxy', { label: s.name }) : t('switched', { label: s.name }));
    } catch (e) { toast(errText(e), true); }
    u.busy = false; render();
  }

  async function doRestore() {
    try {
      const [r, v] = await invoke('restore');
      view = v;
      toast(r === 'NothingToRestore' ? t('restoreNone') : r === 'RemovedAgain' ? t('restoredRemoved') : t('restored'));
    } catch (e) { toast(errText(e), true); }
    render();
  }

  async function doOfficial() {
    try { view = await invoke('use_official'); toast(t('officialDone')); } catch (e) { toast(errText(e), true); }
    render();
  }

  async function refresh() {
    loading = true;
    $('refresh').disabled = true;
    try { view = await invoke('refresh'); } catch (e) { toast(errText(e), true); }
    loading = false;
    $('refresh').disabled = false;
    for (const k of Object.keys(ui)) if (ui[k] && typeof ui[k] === 'object') delete ui[k].models;
    render();
  }

  /* ---------------------------------------------------------------- start */
  $('refresh').addEventListener('click', refresh);
  $('footerWickrunLink').addEventListener('click', (e) => { e.preventDefault(); invoke('open_link', { url: (view && view.download_url) || 'https://wickrunai.com' }); });
  $('autostart').addEventListener('change', async (e) => {
    try { e.target.checked = await invoke('set_autostart', { on: e.target.checked }); } catch (err) { e.target.checked = !e.target.checked; toast(errText(err), true); }
  });
  if (T && T.event) T.event.listen('changed', async () => { view = await invoke('get_view'); render(); });

  (async () => {
    renderStatic();
    view = await invoke('get_view');
    render();
    invoke('get_autostart').then((on) => ($('autostart').checked = !!on)).catch(() => {});
    await refresh();
  })();
})();
