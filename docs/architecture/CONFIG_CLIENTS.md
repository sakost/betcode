# Client Configuration Reference

**Version**: 0.2.0
**Last Updated**: 2026-02-03
**Parent**: [CONFIG_REFERENCE.md](./CONFIG_REFERENCE.md)

---

## CLI Configuration

CLI settings live in `$BETCODE_CONFIG_DIR/settings.json` under the `cli` object.

### Display Settings

| Parameter | Type | Default | CLI Flag |
|-----------|------|---------|----------|
| `cli.theme` | string | "auto" | `--theme` |
| `cli.markdown_rendering` | boolean | true | `--no-markdown` |
| `cli.syntax_highlighting` | boolean | true | `--no-syntax` |
| `cli.show_timestamps` | boolean | false | `--timestamps` |
| `cli.show_token_usage` | boolean | true | `--no-tokens` |

Valid themes: `auto`, `dark`, `light`, `none`

### Connection Settings

| Parameter | Type | Default | Min | Max | CLI Flag |
|-----------|------|---------|-----|-----|----------|
| `cli.prefer_local` | boolean | true | - | - | `--remote` |
| `cli.default_machine_id` | string | null | - | - | `--machine` |
| `cli.connection_timeout_seconds` | integer | 10 | 5 | 60 | - |

### Session Settings

| Parameter | Type | Default | Min | Max | CLI Flag |
|-----------|------|---------|-----|-----|----------|
| `cli.default_model` | string | null | - | - | `--model` |
| `cli.auto_resume` | boolean | false | - | - | `--continue` |
| `cli.max_history_display` | integer | 50 | 10 | 200 | - |

### Headless Mode Settings

| Parameter | Type | Default | CLI Flag |
|-----------|------|---------|----------|
| `cli.headless.output_format` | string | "text" | `--output-format` |
| `cli.headless.max_turns` | integer | 0 | `--max-turns` |
| `cli.headless.timeout_seconds` | integer | 0 | `--timeout` |

Valid output formats: `text`, `json`, `stream-json`

### Keybindings

| Parameter | Default | Description |
|-----------|---------|-------------|
| `cli.keys.send` | "ctrl+enter" | Send message |
| `cli.keys.cancel` | "ctrl+c" | Cancel current turn |
| `cli.keys.scroll_up` | "k" | Scroll conversation up |
| `cli.keys.scroll_down` | "j" | Scroll conversation down |
| `cli.keys.toggle_diff` | "d" | Toggle diff view mode |
| `cli.keys.permission_allow` | "y" | Allow permission |
| `cli.keys.permission_allow_session` | "a" | Allow for session |
| `cli.keys.permission_deny` | "n" | Deny permission |
| `cli.keys.quit` | "ctrl+q" | Quit application |

---

## Flutter Client Configuration

Flutter client settings are stored locally and synchronized from the relay.

### Local Settings (Device-Specific)

| Parameter | Type | Default | Range |
|-----------|------|---------|-------|
| `client.theme_mode` | string | "system" | system, dark, light |
| `client.font_size` | integer | 14 | 12-24 |
| `client.haptic_feedback` | boolean | true | - |
| `client.keep_screen_on` | boolean | false | - |

### Connection Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `client.connection.timeout_seconds` | integer | 30 | 10 | 120 |
| `client.connection.retry_max_attempts` | integer | 5 | 1 | 20 |
| `client.connection.retry_base_delay_ms` | integer | 100 | 50 | 1000 |
| `client.connection.retry_max_delay_ms` | integer | 30000 | 5000 | 60000 |
| `client.connection.stability_delay_ms` | integer | 3000 | 1000 | 10000 |

### Offline Queue Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `client.sync.queue_limit` | integer | 500 | 100 | 2000 |
| `client.sync.retry_base_delay_ms` | integer | 1000 | 500 | 5000 |
| `client.sync.retry_max_delay_ms` | integer | 300000 | 60000 | 600000 |
| `client.sync.retry_max_attempts_interactive` | integer | 5 | 1 | 20 |
| `client.sync.retry_max_attempts_background` | integer | 10 | 1 | 30 |

### Cache Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `client.cache.session_retention_days` | integer | 7 | 1 | 30 |
| `client.cache.max_sessions` | integer | 50 | 10 | 200 |
| `client.cache.notification_id_ttl_hours` | integer | 1 | 1 | 24 |

### Push Notification Preferences (Synced)

| Parameter | Type | Default |
|-----------|------|---------|
| `client.push.permission_requests` | boolean | true |
| `client.push.user_questions` | boolean | true |
| `client.push.errors` | boolean | true |
| `client.push.task_completion` | boolean | false |
| `client.push.session_updates` | boolean | false |

### Rate Limit Client Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `client.rate_limit.jitter_factor` | float | 0.2 | 0.0 | 0.5 |
| `client.rate_limit.warn_threshold` | integer | 3 | 1 | 10 |
