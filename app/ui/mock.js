// Sample data used only when the page is opened outside the app (for screenshots and UI work).
// The real app never loads this data: window.__TAURI__ is present there.
(() => {
  if (window.__TAURI__) return;
  const scenario = new URLSearchParams(location.search).get('state') || 'proxy';
  const src = (o) => Object.assign({ endpoints: [], note: null, docs: null, key_masked: null, models: [], default_model: null, auth: 'bearer' }, o);
  const sources = [
    src({ id: 'local:ollama:11434', group: 'local', name: 'Ollama', origin: 'url:http://127.0.0.1:11434', status: 'reachable', route: 'direct', base_url: 'http://127.0.0.1:11434', models: ['qwen3-coder:30b', 'gpt-oss:20b'], default_model: 'qwen3-coder:30b' }),
    src({ id: 'local:lmstudio:1234', group: 'local', name: 'LM Studio', origin: 'url:http://127.0.0.1:1234', status: 'reachable', route: 'direct', base_url: 'http://127.0.0.1:1234', models: ['qwen/qwen3-coder-30b'], default_model: 'qwen/qwen3-coder-30b' }),
    src({ id: 'key:deepseek:env:DEEPSEEK_API_KEY', group: 'keys', name: 'DeepSeek', origin: 'env:DEEPSEEK_API_KEY', status: 'reachable', route: 'direct', base_url: 'https://api.deepseek.com/anthropic', auth: 'api_key', key_masked: '…3f9a', models: ['deepseek-v4-pro', 'deepseek-flash'], default_model: 'deepseek-v4-pro', docs: 'https://api-docs.deepseek.com/guides/anthropic_api' }),
    src({ id: 'key:moonshot:file', group: 'keys', name: 'Kimi (Moonshot)', origin: 'file:~/.zshrc#MOONSHOT_API_KEY', status: 'reachable', route: 'direct', base_url: 'https://api.moonshot.cn/anthropic', key_masked: '…k2Qx', models: ['kimi-k3.1-pro', 'kimi-k3'], default_model: 'kimi-k3.1-pro', endpoints: [['china', 'https://api.moonshot.cn/anthropic'], ['global', 'https://api.moonshot.ai/anthropic']] }),
    src({ id: 'key:openai:env:OPENAI_API_KEY', group: 'keys', name: 'OpenAI', origin: 'env:OPENAI_API_KEY', status: 'reachable', route: 'translate', base_url: 'https://api.openai.com/v1', key_masked: '…T0aB', models: ['gpt-5.2', 'gpt-5.2-mini'], default_model: 'gpt-5.2', note: 'via_translator' }),
    src({ id: 'ccswitch:deepseek-1', group: 'ccswitch', name: 'GLM Coding Plan', origin: 'file:~/.cc-switch/cc-switch.db', status: 'reachable', route: 'direct', base_url: 'https://open.bigmodel.cn/api/anthropic', key_masked: '…cc01', models: ['glm-5.3'], default_model: 'glm-5.3', note: 'cc_current' }),
    src({ id: 'ccswitch:official', group: 'ccswitch', name: 'Claude Official', origin: 'file:~/.cc-switch/cc-switch.db', status: 'not_compatible', route: 'none', base_url: '', note: 'cc_official' }),
    src({ id: 'gateway:omniroute:20128', group: 'gateway', name: 'OmniRoute', origin: 'url:http://127.0.0.1:20128', status: 'reachable', route: 'direct', base_url: 'http://127.0.0.1:20128', models: ['kr/claude-sonnet-4.5', 'gemini/gemini-3-pro'], default_model: 'kr/claude-sonnet-4.5' }),
    src({ id: 'gateway:litellm:4000', group: 'gateway', name: 'LiteLLM', origin: 'url:http://127.0.0.1:4000', status: 'needs_key', route: 'direct', base_url: 'http://127.0.0.1:4000' }),
    src({ id: 'wickrunai', group: 'wickrunai', name: 'wickrunAI', origin: 'file:/Applications/wickrunAI.app', status: 'managed_elsewhere', route: 'none', base_url: '', note: 'wickrunai_detected' }),
  ];
  const currents = {
    official: { kind: 'official', label: null, model: null, backup_exists: false, base_url: null },
    direct: { kind: 'custom', label: 'DeepSeek', model: 'deepseek-v4-pro', backup_exists: true, base_url: 'https://api.deepseek.com/anthropic', credential_var: 'ANTHROPIC_API_KEY', credential_masked: '…3f9a' },
    proxy: { kind: 'proxy', label: 'OpenAI', model: 'gpt-5.2', backup_exists: true, base_url: 'http://127.0.0.1:41873', credential_var: 'ANTHROPIC_AUTH_TOKEN', credential_masked: '…9c2e' },
    down: { kind: 'proxy_down', label: 'OpenAI', model: 'gpt-5.2', backup_exists: true, base_url: 'http://127.0.0.1:41873', credential_var: 'ANTHROPIC_AUTH_TOKEN', credential_masked: '…9c2e' },
    wickrunai: { kind: 'wickrunai', label: null, model: 'kr/claude-sonnet-4.5', backup_exists: false, base_url: 'http://127.0.0.1:45123' },
  };
  const active = { direct: 'key:deepseek:env:DEEPSEEK_API_KEY', proxy: 'key:openai:env:OPENAI_API_KEY', down: 'key:openai:env:OPENAI_API_KEY' };
  const view = () => ({
    current: Object.assign({ settings_file: '~/.claude/settings.json', credential_var: null, credential_masked: null }, currents[scenario] || currents.proxy),
    active_id: active[scenario] || null,
    sources,
    proxy_running: scenario === 'proxy',
    proxy_port: 41873,
    proxy_error: null,
    wickrunai_installed: true,
    download_url: 'https://wickrunai.com',
    version: '0.2.0',
  });
  window.WB_MOCK = {
    async invoke(cmd, args) {
      switch (cmd) {
        case 'get_view': case 'refresh': return view();
        case 'list_models': return (sources.find((s) => s.id === args.id) || {}).models || [];
        case 'test_source': return { ok: true, status: 200, ms: 842 };
        case 'get_autostart': return false;
        default: return view();
      }
    },
  };
})();
