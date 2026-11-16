CREATE TABLE providers (id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL, settings_config TEXT NOT NULL, website_url TEXT, category TEXT, created_at INTEGER, sort_index INTEGER, notes TEXT, icon TEXT, icon_color TEXT, meta TEXT NOT NULL DEFAULT '{}', is_current BOOLEAN NOT NULL DEFAULT 0, in_failover_queue BOOLEAN NOT NULL DEFAULT 0, PRIMARY KEY (id, app_type));
INSERT INTO providers (id, app_type, name, settings_config, sort_index, meta, is_current) VALUES
 ('official','claude','Claude Official','{"env":{}}',0,'{}',0),
 ('deepseek-1','claude','DeepSeek','{"env":{"ANTHROPIC_BASE_URL":"https://api.deepseek.com/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-ccswitch-deepseek-key-0000cc01","ANTHROPIC_MODEL":"deepseek-v4-pro"}}',1,'{}',1),
 ('openai-chat','claude','My OpenAI-format relay','{"env":{"ANTHROPIC_BASE_URL":"https://relay.example.com/v1","ANTHROPIC_AUTH_TOKEN":"sk-relay-0123456789abcdef","ANTHROPIC_MODEL":"gpt-5"}}',2,'{"apiFormat":"openai_chat"}',0),
 ('nokey','claude','Half configured','{"env":{"ANTHROPIC_BASE_URL":"https://open.bigmodel.cn/api/anthropic"}}',3,'{}',0),
 ('codex-1','codex','A Codex provider','{"auth":{"OPENAI_API_KEY":"sk-codex"}}',0,'{}',1);
