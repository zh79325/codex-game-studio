use crate::StoreError;
use codex_game_domain::AiCapability;
use codex_game_domain::AiProvider;
use codex_game_domain::BreakerState;
use codex_game_domain::LimitKind;
use codex_game_domain::LimitPolicy;
use codex_game_domain::ModelUsage;
use codex_game_domain::ProviderModel;
use codex_game_domain::UsageBudget;
use sqlx::Row;
use sqlx::SqlitePool;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRouteModel {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub capabilities: Vec<AiCapability>,
    pub available: bool,
}

const AI_CONFIG_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS ai_config_migrations (
    name TEXT PRIMARY KEY,
    applied_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS ai_providers (
    code TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    driver TEXT NOT NULL,
    auth_style TEXT NOT NULL DEFAULT 'bearer',
    priority INTEGER NOT NULL,
    enabled INTEGER NOT NULL,
    remark TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS ai_models (
    id TEXT PRIMARY KEY,
    provider_code TEXT NOT NULL,
    model_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    driver TEXT NOT NULL,
    api_path TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    sort_no INTEGER NOT NULL,
    params_json TEXT NOT NULL,
    remark TEXT NOT NULL,
    UNIQUE(provider_code, model_id)
);
CREATE TABLE IF NOT EXISTS ai_agent_bindings (
    agent_code TEXT NOT NULL,
    model_id TEXT NOT NULL,
    sort_no INTEGER NOT NULL,
    PRIMARY KEY(agent_code, model_id)
);
CREATE TABLE IF NOT EXISTS ai_limits (
    model_id TEXT NOT NULL,
    limit_kind TEXT NOT NULL,
    max_value INTEGER NOT NULL,
    period_expr TEXT NOT NULL,
    group_name TEXT NOT NULL,
    PRIMARY KEY(model_id, limit_kind)
);
CREATE TABLE IF NOT EXISTS ai_usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    model_id TEXT NOT NULL,
    limit_kind TEXT NOT NULL,
    amount INTEGER NOT NULL,
    idempotency_key TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(model_id, limit_kind, idempotency_key)
);
CREATE TABLE IF NOT EXISTS ai_breakers (
    model_id TEXT PRIMARY KEY,
    failure_count INTEGER NOT NULL,
    last_reason TEXT,
    opened_at INTEGER,
    retry_at INTEGER
);
"#;

pub async fn initialize_ai_config_schema(pool: &SqlitePool) -> Result<(), StoreError> {
    for statement in AI_CONFIG_SCHEMA
        .split(';')
        .map(str::trim)
        .filter(|sql| !sql.is_empty())
    {
        sqlx::query(statement).execute(pool).await?;
    }
    migrate_legacy_ai_data(pool).await?;
    migrate_provider_metadata(pool).await?;
    Ok(())
}

async fn migrate_provider_metadata(pool: &SqlitePool) -> Result<(), StoreError> {
    let columns = sqlx::query("PRAGMA table_info(ai_providers)")
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<std::collections::HashSet<_>>();
    if !columns.contains("auth_style") {
        sqlx::query(
            "ALTER TABLE ai_providers ADD COLUMN auth_style TEXT NOT NULL DEFAULT 'bearer'",
        )
        .execute(pool)
        .await?;
    }
    if !columns.contains("remark") {
        sqlx::query("ALTER TABLE ai_providers ADD COLUMN remark TEXT NOT NULL DEFAULT ''")
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn migrate_legacy_ai_data(pool: &SqlitePool) -> Result<(), StoreError> {
    const MIGRATION_NAME: &str = "legacy-ai-config-v1";
    let mut transaction = pool.begin().await?;
    let applied: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ai_config_migrations WHERE name = ?)")
            .bind(MIGRATION_NAME)
            .fetch_one(&mut *transaction)
            .await?;
    if applied {
        transaction.rollback().await?;
        return Ok(());
    }
    sqlx::query(
        "INSERT OR IGNORE INTO ai_providers(code, name, base_url, driver, priority, enabled) SELECT provider, provider, '', 'openai', rowid, enabled FROM provider_accounts WHERE provider <> ''",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO ai_models(id, provider_code, model_id, display_name, capabilities_json, driver, api_path, enabled, sort_no, params_json, remark) SELECT id, provider, model, model, '[\"text_reasoning\",\"text_structured_output\"]', 'openai', '/v1/responses', enabled, rowid, '{}', 'Migrated from provider_accounts' FROM provider_accounts WHERE provider <> '' AND model <> ''",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO ai_usage(model_id, limit_kind, amount, idempotency_key, created_at) SELECT provider_account_id, CASE metric WHEN 'requests' THEN 'calls' ELSE metric END, amount, idempotency_key, created_at FROM usage_ledger WHERE EXISTS(SELECT 1 FROM ai_models WHERE id = usage_ledger.provider_account_id)",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO ai_config_migrations(name, applied_at) VALUES (?, ?)")
        .bind(MIGRATION_NAME)
        .bind(current_timestamp())
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn list_ai_providers(pool: &SqlitePool) -> Result<Vec<AiProvider>, StoreError> {
    let provider_rows = sqlx::query(
        "SELECT code, name, base_url, driver, auth_style, priority, enabled, remark FROM ai_providers ORDER BY priority, code",
    )
    .fetch_all(pool)
    .await?;
    let mut providers = Vec::with_capacity(provider_rows.len());
    for row in provider_rows {
        let code: String = row.try_get("code")?;
        providers.push(AiProvider {
            models: list_models(pool, &code).await?,
            code,
            name: row.try_get("name")?,
            base_url: row.try_get("base_url")?,
            driver: row.try_get("driver")?,
            auth_style: row.try_get("auth_style")?,
            priority: row.try_get("priority")?,
            enabled: row.try_get("enabled")?,
            remark: row.try_get("remark")?,
            has_key: false,
            key_mask: None,
        });
    }
    Ok(providers)
}

pub async fn load_ai_route_models(
    pool: &SqlitePool,
    agent_code: &str,
    now: i64,
) -> Result<Vec<AiRouteModel>, StoreError> {
    let rows = sqlx::query(
        "SELECT m.id, m.provider_code, m.model_id, m.capabilities_json, p.enabled AS provider_enabled, m.enabled AS model_enabled, b.retry_at FROM ai_agent_bindings ab JOIN ai_models m ON m.id = ab.model_id JOIN ai_providers p ON p.code = m.provider_code LEFT JOIN ai_breakers b ON b.model_id = m.id WHERE ab.agent_code = ? ORDER BY ab.sort_no",
    )
    .bind(agent_code)
    .fetch_all(pool)
    .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id")?;
        let has_capacity = model_has_capacity(pool, &id, now).await?;
        let retry_at: Option<i64> = row.try_get("retry_at")?;
        result.push(AiRouteModel {
            id,
            provider: row.try_get("provider_code")?,
            model: row.try_get("model_id")?,
            capabilities: serde_json::from_str(&row.try_get::<String, _>("capabilities_json")?)?,
            available: row.try_get::<bool, _>("provider_enabled")?
                && row.try_get::<bool, _>("model_enabled")?
                && retry_at.is_none_or(|retry_at| retry_at <= now)
                && has_capacity,
        });
    }
    Ok(result)
}

pub async fn reserve_ai_usage(
    pool: &SqlitePool,
    model_id: &str,
    idempotency_key: &str,
    requirements: &[(LimitKind, u64)],
    now: i64,
) -> Result<bool, StoreError> {
    let mut transaction = pool.begin().await?;
    let already_recorded: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM ai_usage WHERE model_id = ? AND idempotency_key = ?)",
    )
    .bind(model_id)
    .bind(idempotency_key)
    .fetch_one(&mut *transaction)
    .await?;
    if already_recorded {
        transaction.rollback().await?;
        return Ok(false);
    }
    for (kind, amount) in requirements {
        let kind_name = limit_kind_name(kind);
        let limit = sqlx::query(
            "SELECT max_value, period_expr, group_name FROM ai_limits WHERE model_id = ? AND limit_kind = ?",
        )
        .bind(model_id)
        .bind(kind_name)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(limit) = limit {
            let max_value = limit.try_get::<i64, _>("max_value")? as u64;
            let period_expr: String = limit.try_get("period_expr")?;
            let group_name: String = limit.try_get("group_name")?;
            if max_value > 0 {
                let window_start = period_start(now, &period_expr)?;
                let used: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(SUM(u.amount), 0) FROM ai_usage u JOIN ai_limits l ON l.model_id = u.model_id AND l.limit_kind = u.limit_kind WHERE l.limit_kind = ? AND l.group_name = ? AND l.period_expr = ? AND u.created_at >= ?",
                )
                .bind(kind_name)
                .bind(&group_name)
                .bind(&period_expr)
                .bind(window_start)
                .fetch_one(&mut *transaction)
                .await?;
                if used as u64 + amount > max_value {
                    transaction.rollback().await?;
                    return Err(StoreError::Conflict(format!(
                        "AI quota exceeded for model {model_id}, metric {kind_name}"
                    )));
                }
            }
        }
    }
    for (kind, amount) in requirements {
        sqlx::query(
            "INSERT INTO ai_usage(model_id, limit_kind, amount, idempotency_key, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(model_id)
        .bind(limit_kind_name(kind))
        .bind(*amount as i64)
        .bind(idempotency_key)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(true)
}

pub async fn record_ai_route_success(pool: &SqlitePool, model_id: &str) -> Result<(), StoreError> {
    clear_ai_breaker(pool, model_id).await
}

pub async fn record_ai_route_failure(
    pool: &SqlitePool,
    model_id: &str,
    reason: &str,
    now: i64,
) -> Result<(), StoreError> {
    let params_json: Option<String> =
        sqlx::query_scalar("SELECT params_json FROM ai_models WHERE id = ?")
            .bind(model_id)
            .fetch_optional(pool)
            .await?;
    let params = params_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .unwrap_or_default();
    let threshold = params
        .get("breakerThreshold")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(3)
        .max(1);
    let cooldown = params
        .get("breakerCooldownSeconds")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(60);
    let previous: Option<i64> =
        sqlx::query_scalar("SELECT failure_count FROM ai_breakers WHERE model_id = ?")
            .bind(model_id)
            .fetch_optional(pool)
            .await?;
    let failure_count = previous.unwrap_or(0) + 1;
    let (opened_at, retry_at) = if failure_count as u64 >= threshold {
        (Some(now), Some(now.saturating_add(cooldown as i64)))
    } else {
        (None, None)
    };
    sqlx::query(
        "INSERT INTO ai_breakers(model_id, failure_count, last_reason, opened_at, retry_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(model_id) DO UPDATE SET failure_count = excluded.failure_count, last_reason = excluded.last_reason, opened_at = excluded.opened_at, retry_at = excluded.retry_at",
    )
    .bind(model_id)
    .bind(failure_count)
    .bind(reason)
    .bind(opened_at)
    .bind(retry_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn replace_ai_configuration(
    pool: &SqlitePool,
    providers: &[AiProvider],
    bindings: &HashMap<String, Vec<String>>,
) -> Result<(), StoreError> {
    let model_ids = providers
        .iter()
        .flat_map(|provider| provider.models.iter())
        .map(|model| model.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    for model_id in bindings.values().flatten() {
        if !model_ids.contains(model_id.as_str()) {
            return Err(StoreError::NotFound(format!("AI model {model_id}")));
        }
    }
    let mut transaction = pool.begin().await?;
    for statement in [
        "DELETE FROM ai_agent_bindings",
        "DELETE FROM ai_limits",
        "DELETE FROM ai_breakers",
        "DELETE FROM ai_models",
        "DELETE FROM ai_providers",
    ] {
        sqlx::query(statement).execute(&mut *transaction).await?;
    }
    for provider in providers {
        sqlx::query(
            "INSERT INTO ai_providers(code, name, base_url, driver, auth_style, priority, enabled, remark) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&provider.code)
        .bind(&provider.name)
        .bind(&provider.base_url)
        .bind(&provider.driver)
        .bind(&provider.auth_style)
        .bind(provider.priority)
        .bind(provider.enabled)
        .bind(&provider.remark)
        .execute(&mut *transaction)
        .await?;
        for model in &provider.models {
            sqlx::query(
                "INSERT INTO ai_models(id, provider_code, model_id, display_name, capabilities_json, driver, api_path, enabled, sort_no, params_json, remark) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&model.id)
            .bind(&provider.code)
            .bind(&model.model_id)
            .bind(&model.display_name)
            .bind(serde_json::to_string(&model.capabilities)?)
            .bind(&model.driver)
            .bind(&model.api_path)
            .bind(model.enabled)
            .bind(model.sort_no)
            .bind(serde_json::to_string(&model.params)?)
            .bind(&model.remark)
            .execute(&mut *transaction)
            .await?;
            for limit in &model.limits {
                sqlx::query(
                    "INSERT INTO ai_limits(model_id, limit_kind, max_value, period_expr, group_name) VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&model.id)
                .bind(limit_kind_name(&limit.limit_kind))
                .bind(limit.max_value as i64)
                .bind(&limit.period_expr)
                .bind(&limit.group_name)
                .execute(&mut *transaction)
                .await?;
            }
        }
    }
    for (agent_code, model_ids) in bindings {
        for (sort_no, model_id) in model_ids.iter().enumerate() {
            sqlx::query(
                "INSERT INTO ai_agent_bindings(agent_code, model_id, sort_no) VALUES (?, ?, ?)",
            )
            .bind(agent_code)
            .bind(model_id)
            .bind(sort_no as i64)
            .execute(&mut *transaction)
            .await?;
        }
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn create_ai_provider_configuration(
    pool: &SqlitePool,
    provider: &AiProvider,
    bindings: &HashMap<String, Vec<String>>,
) -> Result<(), StoreError> {
    let mut transaction = pool.begin().await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ai_providers WHERE code = ?)")
            .bind(&provider.code)
            .fetch_one(&mut *transaction)
            .await?;
    if exists {
        return Err(StoreError::Conflict(format!(
            "AI provider {} already exists",
            provider.code
        )));
    }
    sqlx::query(
        "INSERT INTO ai_providers(code, name, base_url, driver, auth_style, priority, enabled, remark) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&provider.code)
    .bind(&provider.name)
    .bind(&provider.base_url)
    .bind(&provider.driver)
    .bind(&provider.auth_style)
    .bind(provider.priority)
    .bind(provider.enabled)
    .bind(&provider.remark)
    .execute(&mut *transaction)
    .await?;
    let mut model_ids = std::collections::HashSet::new();
    let mut provider_model_ids = std::collections::HashSet::new();
    for model in &provider.models {
        if model.provider_code != provider.code
            || !model_ids.insert(model.id.as_str())
            || !provider_model_ids.insert(model.model_id.as_str())
        {
            return Err(StoreError::InvalidData(format!(
                "invalid or duplicate model {} for provider {}",
                model.model_id, provider.code
            )));
        }
        sqlx::query(
            "INSERT INTO ai_models(id, provider_code, model_id, display_name, capabilities_json, driver, api_path, enabled, sort_no, params_json, remark) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&model.id)
        .bind(&model.provider_code)
        .bind(&model.model_id)
        .bind(&model.display_name)
        .bind(serde_json::to_string(&model.capabilities)?)
        .bind(&model.driver)
        .bind(&model.api_path)
        .bind(model.enabled)
        .bind(model.sort_no)
        .bind(serde_json::to_string(&model.params)?)
        .bind(&model.remark)
        .execute(&mut *transaction)
        .await?;
        for limit in &model.limits {
            sqlx::query(
                "INSERT INTO ai_limits(model_id, limit_kind, max_value, period_expr, group_name) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&model.id)
            .bind(limit_kind_name(&limit.limit_kind))
            .bind(limit.max_value as i64)
            .bind(&limit.period_expr)
            .bind(&limit.group_name)
            .execute(&mut *transaction)
            .await?;
        }
    }
    for (agent_code, bound_model_ids) in bindings {
        let first_sort_no: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sort_no) + 1, 0) FROM ai_agent_bindings WHERE agent_code = ?",
        )
        .bind(agent_code)
        .fetch_one(&mut *transaction)
        .await?;
        for (index, model_id) in bound_model_ids.iter().enumerate() {
            if !model_ids.contains(model_id.as_str()) {
                return Err(StoreError::NotFound(format!("AI model {model_id}")));
            }
            sqlx::query(
                "INSERT INTO ai_agent_bindings(agent_code, model_id, sort_no) VALUES (?, ?, ?)",
            )
            .bind(agent_code)
            .bind(model_id)
            .bind(first_sort_no.saturating_add(index as i64))
            .execute(&mut *transaction)
            .await?;
        }
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn upsert_ai_provider(
    pool: &SqlitePool,
    provider: &AiProvider,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO ai_providers(code, name, base_url, driver, auth_style, priority, enabled, remark) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(code) DO UPDATE SET name = excluded.name, base_url = excluded.base_url, driver = excluded.driver, auth_style = excluded.auth_style, priority = excluded.priority, enabled = excluded.enabled, remark = excluded.remark",
    )
    .bind(&provider.code)
    .bind(&provider.name)
    .bind(&provider.base_url)
    .bind(&provider.driver)
    .bind(&provider.auth_style)
    .bind(provider.priority)
    .bind(provider.enabled)
    .bind(&provider.remark)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_ai_provider(pool: &SqlitePool, code: &str) -> Result<(), StoreError> {
    let model_ids =
        sqlx::query_scalar::<_, String>("SELECT id FROM ai_models WHERE provider_code = ?")
            .bind(code)
            .fetch_all(pool)
            .await?;
    for model_id in &model_ids {
        ensure_model_unbound(pool, model_id).await?;
    }
    let mut transaction = pool.begin().await?;
    for model_id in model_ids {
        sqlx::query("DELETE FROM ai_limits WHERE model_id = ?")
            .bind(&model_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM ai_breakers WHERE model_id = ?")
            .bind(&model_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM ai_usage WHERE model_id = ?")
            .bind(&model_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM ai_models WHERE id = ?")
            .bind(&model_id)
            .execute(&mut *transaction)
            .await?;
    }
    sqlx::query("DELETE FROM ai_providers WHERE code = ?")
        .bind(code)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn upsert_ai_model(pool: &SqlitePool, model: &ProviderModel) -> Result<(), StoreError> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO ai_models(id, provider_code, model_id, display_name, capabilities_json, driver, api_path, enabled, sort_no, params_json, remark) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET provider_code = excluded.provider_code, model_id = excluded.model_id, display_name = excluded.display_name, capabilities_json = excluded.capabilities_json, driver = excluded.driver, api_path = excluded.api_path, enabled = excluded.enabled, sort_no = excluded.sort_no, params_json = excluded.params_json, remark = excluded.remark",
    )
    .bind(&model.id)
    .bind(&model.provider_code)
    .bind(&model.model_id)
    .bind(&model.display_name)
    .bind(serde_json::to_string(&model.capabilities)?)
    .bind(&model.driver)
    .bind(&model.api_path)
    .bind(model.enabled)
    .bind(model.sort_no)
    .bind(serde_json::to_string(&model.params)?)
    .bind(&model.remark)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM ai_limits WHERE model_id = ?")
        .bind(&model.id)
        .execute(&mut *transaction)
        .await?;
    for limit in &model.limits {
        sqlx::query(
            "INSERT INTO ai_limits(model_id, limit_kind, max_value, period_expr, group_name) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&model.id)
        .bind(limit_kind_name(&limit.limit_kind))
        .bind(limit.max_value as i64)
        .bind(&limit.period_expr)
        .bind(&limit.group_name)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn delete_ai_model(pool: &SqlitePool, model_id: &str) -> Result<(), StoreError> {
    ensure_model_unbound(pool, model_id).await?;
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM ai_limits WHERE model_id = ?")
        .bind(model_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM ai_breakers WHERE model_id = ?")
        .bind(model_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM ai_usage WHERE model_id = ?")
        .bind(model_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM ai_models WHERE id = ?")
        .bind(model_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn list_agent_bindings(
    pool: &SqlitePool,
) -> Result<HashMap<String, Vec<String>>, StoreError> {
    let rows = sqlx::query(
        "SELECT agent_code, model_id FROM ai_agent_bindings ORDER BY agent_code, sort_no",
    )
    .fetch_all(pool)
    .await?;
    let mut bindings = HashMap::<String, Vec<String>>::new();
    for row in rows {
        bindings
            .entry(row.try_get("agent_code")?)
            .or_default()
            .push(row.try_get("model_id")?);
    }
    Ok(bindings)
}

pub async fn write_agent_binding(
    pool: &SqlitePool,
    agent_code: &str,
    model_ids: &[String],
) -> Result<(), StoreError> {
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM ai_agent_bindings WHERE agent_code = ?")
        .bind(agent_code)
        .execute(&mut *transaction)
        .await?;
    for (sort_no, model_id) in model_ids.iter().enumerate() {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ai_models WHERE id = ?)")
                .bind(model_id)
                .fetch_one(&mut *transaction)
                .await?;
        if !exists {
            return Err(StoreError::NotFound(format!("AI model {model_id}")));
        }
        sqlx::query(
            "INSERT INTO ai_agent_bindings(agent_code, model_id, sort_no) VALUES (?, ?, ?)",
        )
        .bind(agent_code)
        .bind(model_id)
        .bind(sort_no as i64)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn read_ai_usage(pool: &SqlitePool) -> Result<Vec<ModelUsage>, StoreError> {
    let providers = list_ai_providers(pool).await?;
    let bindings = list_agent_bindings(pool).await?;
    let mut items = Vec::new();
    for provider in providers {
        for model in provider.models {
            let agents = bindings
                .iter()
                .filter(|(_, models)| models.contains(&model.id))
                .map(|(agent, _)| agent.clone())
                .collect();
            let mut budgets = Vec::new();
            for limit in &model.limits {
                let window_start = period_start(current_timestamp(), &limit.period_expr)?;
                let used = grouped_usage_total(pool, limit, window_start).await?;
                budgets.push(UsageBudget {
                    limit_kind: limit.limit_kind.clone(),
                    used,
                    limit: limit.max_value,
                    period_expr: limit.period_expr.clone(),
                    window_key: window_start.to_string(),
                    group_name: limit.group_name.clone(),
                    source: "local".to_string(),
                    exhausted: limit.max_value > 0 && used >= limit.max_value,
                    unlimited: limit.max_value == 0,
                });
            }
            items.push(ModelUsage {
                provider_code: provider.code.clone(),
                provider_name: provider.name.clone(),
                provider_model_id: model.id.clone(),
                model_id: model.model_id,
                provider_enabled: provider.enabled,
                enabled: model.enabled,
                has_key: provider.has_key,
                agents,
                budgets,
                breaker: read_breaker(pool, &model.id).await?,
            });
        }
    }
    Ok(items)
}

pub async fn reset_ai_usage(
    pool: &SqlitePool,
    model_id: &str,
    limit_kind: Option<&LimitKind>,
) -> Result<u64, StoreError> {
    let now = current_timestamp();
    let result = if let Some(limit_kind) = limit_kind {
        let period_expr: Option<String> = sqlx::query_scalar(
            "SELECT period_expr FROM ai_limits WHERE model_id = ? AND limit_kind = ?",
        )
        .bind(model_id)
        .bind(limit_kind_name(limit_kind))
        .fetch_optional(pool)
        .await?;
        let window_start = period_expr
            .as_deref()
            .map(|period| period_start(now, period))
            .transpose()?
            .unwrap_or(0);
        sqlx::query(
            "DELETE FROM ai_usage WHERE model_id = ? AND limit_kind = ? AND created_at >= ?",
        )
        .bind(model_id)
        .bind(limit_kind_name(limit_kind))
        .bind(window_start)
        .execute(pool)
        .await?
    } else {
        sqlx::query("DELETE FROM ai_usage WHERE model_id = ?")
            .bind(model_id)
            .execute(pool)
            .await?
    };
    Ok(result.rows_affected())
}

pub async fn clear_ai_breaker(pool: &SqlitePool, model_id: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM ai_breakers WHERE model_id = ?")
        .bind(model_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn list_models(
    pool: &SqlitePool,
    provider_code: &str,
) -> Result<Vec<ProviderModel>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, provider_code, model_id, display_name, capabilities_json, driver, api_path, enabled, sort_no, params_json, remark FROM ai_models WHERE provider_code = ? ORDER BY sort_no, model_id",
    )
    .bind(provider_code)
    .fetch_all(pool)
    .await?;
    let mut models = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id")?;
        models.push(ProviderModel {
            limits: list_limits(pool, &id).await?,
            id,
            provider_code: row.try_get("provider_code")?,
            model_id: row.try_get("model_id")?,
            display_name: row.try_get("display_name")?,
            capabilities: serde_json::from_str(&row.try_get::<String, _>("capabilities_json")?)?,
            driver: row.try_get("driver")?,
            api_path: row.try_get("api_path")?,
            enabled: row.try_get("enabled")?,
            sort_no: row.try_get("sort_no")?,
            params: serde_json::from_str(&row.try_get::<String, _>("params_json")?)?,
            remark: row.try_get("remark")?,
        });
    }
    Ok(models)
}

async fn list_limits(pool: &SqlitePool, model_id: &str) -> Result<Vec<LimitPolicy>, StoreError> {
    sqlx::query(
        "SELECT limit_kind, max_value, period_expr, group_name FROM ai_limits WHERE model_id = ? ORDER BY limit_kind",
    )
    .bind(model_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        Ok(LimitPolicy {
            limit_kind: parse_limit_kind(&row.try_get::<String, _>("limit_kind")?)?,
            max_value: row.try_get::<i64, _>("max_value")? as u64,
            period_expr: row.try_get("period_expr")?,
            group_name: row.try_get("group_name")?,
        })
    })
    .collect()
}

async fn read_breaker(
    pool: &SqlitePool,
    model_id: &str,
) -> Result<Option<BreakerState>, StoreError> {
    sqlx::query(
        "SELECT failure_count, last_reason, opened_at, retry_at FROM ai_breakers WHERE model_id = ?",
    )
    .bind(model_id)
    .fetch_optional(pool)
    .await?
    .map(|row| {
        Ok(BreakerState {
            failure_count: row.try_get::<i64, _>("failure_count")? as u32,
            last_reason: row.try_get("last_reason")?,
            opened_at: row.try_get("opened_at")?,
            retry_at: row.try_get("retry_at")?,
        })
    })
    .transpose()
}

async fn model_has_capacity(
    pool: &SqlitePool,
    model_id: &str,
    now: i64,
) -> Result<bool, StoreError> {
    for limit in list_limits(pool, model_id).await? {
        if limit.max_value > 0
            && grouped_usage_total(pool, &limit, period_start(now, &limit.period_expr)?).await?
                >= limit.max_value
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn grouped_usage_total(
    pool: &SqlitePool,
    limit: &LimitPolicy,
    window_start: i64,
) -> Result<u64, StoreError> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(u.amount), 0) FROM ai_usage u JOIN ai_limits l ON l.model_id = u.model_id AND l.limit_kind = u.limit_kind WHERE l.limit_kind = ? AND l.group_name = ? AND l.period_expr = ? AND u.created_at >= ?",
    )
    .bind(limit_kind_name(&limit.limit_kind))
    .bind(&limit.group_name)
    .bind(&limit.period_expr)
    .bind(window_start)
    .fetch_one(pool)
    .await?;
    Ok(total as u64)
}

fn period_start(now: i64, period: &str) -> Result<i64, StoreError> {
    if period == "total" {
        return Ok(0);
    }
    let (period, offset) = if let Some((period, offset)) = period.split_once('+') {
        (period, parse_period_duration(offset)?)
    } else {
        (period, 0)
    };
    let seconds = match period {
        "second" => 1,
        "minute" => 60,
        "hour" => 60 * 60,
        "day" => 24 * 60 * 60,
        "week" => 7 * 24 * 60 * 60,
        "month" => 30 * 24 * 60 * 60,
        _ => parse_period_duration(period)?,
    };
    Ok(now - (now - offset).rem_euclid(seconds))
}

fn parse_period_duration(period: &str) -> Result<i64, StoreError> {
    let (amount, unit) = period.split_at(period.len().saturating_sub(1));
    let amount = amount
        .parse::<i64>()
        .map_err(|_| StoreError::InvalidData(format!("invalid AI limit period: {period}")))?;
    let multiplier = match unit.to_ascii_lowercase().as_str() {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        "w" => 7 * 24 * 60 * 60,
        _ => {
            return Err(StoreError::InvalidData(format!(
                "invalid AI limit period: {period}"
            )));
        }
    };
    amount
        .checked_mul(multiplier)
        .filter(|value| *value > 0)
        .ok_or_else(|| StoreError::InvalidData(format!("invalid AI limit period: {period}")))
}

fn current_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

async fn ensure_model_unbound(pool: &SqlitePool, model_id: &str) -> Result<(), StoreError> {
    let agent: Option<String> =
        sqlx::query_scalar("SELECT agent_code FROM ai_agent_bindings WHERE model_id = ? LIMIT 1")
            .bind(model_id)
            .fetch_optional(pool)
            .await?;
    if let Some(agent) = agent {
        return Err(StoreError::Conflict(format!(
            "AI model {model_id} is bound to agent {agent}"
        )));
    }
    Ok(())
}

fn limit_kind_name(kind: &LimitKind) -> &'static str {
    match kind {
        LimitKind::Calls => "calls",
        LimitKind::InputTokens => "input_tokens",
        LimitKind::OutputTokens => "output_tokens",
        LimitKind::TotalTokens => "total_tokens",
        LimitKind::Tokens => "tokens",
        LimitKind::Credits => "credits",
    }
}

fn parse_limit_kind(value: &str) -> Result<LimitKind, StoreError> {
    match value {
        "calls" => Ok(LimitKind::Calls),
        "input_tokens" => Ok(LimitKind::InputTokens),
        "output_tokens" => Ok(LimitKind::OutputTokens),
        "total_tokens" => Ok(LimitKind::TotalTokens),
        "tokens" => Ok(LimitKind::Tokens),
        "credits" => Ok(LimitKind::Credits),
        other => Err(StoreError::InvalidData(format!(
            "unknown limit kind: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    #[expect(
        clippy::disallowed_methods,
        reason = "an isolated in-memory SQLite pool is appropriate for store tests"
    )]
    async fn legacy_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open test database");
        sqlx::query(
            "CREATE TABLE provider_accounts(id TEXT PRIMARY KEY, provider TEXT NOT NULL, model TEXT NOT NULL, enabled INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("create legacy providers");
        sqlx::query(
            "CREATE TABLE usage_ledger(id INTEGER PRIMARY KEY AUTOINCREMENT, provider_account_id TEXT NOT NULL, metric TEXT NOT NULL, amount INTEGER NOT NULL, idempotency_key TEXT NOT NULL, created_at INTEGER NOT NULL, UNIQUE(provider_account_id, metric, idempotency_key))",
        )
        .execute(&pool)
        .await
        .expect("create legacy usage");
        pool
    }

    async fn test_pool() -> SqlitePool {
        let pool = legacy_pool().await;
        initialize_ai_config_schema(&pool)
            .await
            .expect("initialize AI schema");
        pool
    }

    fn model(id: &str, provider_code: &str, limit: u64, group: &str) -> ProviderModel {
        ProviderModel {
            id: id.to_string(),
            provider_code: provider_code.to_string(),
            model_id: format!("remote-{id}"),
            display_name: id.to_string(),
            capabilities: vec![AiCapability::TextReasoning],
            driver: "openai".to_string(),
            api_path: "/v1/responses".to_string(),
            enabled: true,
            sort_no: 0,
            params: json!({}),
            remark: String::new(),
            limits: vec![LimitPolicy {
                limit_kind: LimitKind::Calls,
                max_value: limit,
                period_expr: "day".to_string(),
                group_name: group.to_string(),
            }],
        }
    }

    fn provider(code: &str, models: Vec<ProviderModel>) -> AiProvider {
        AiProvider {
            code: code.to_string(),
            name: code.to_string(),
            base_url: String::new(),
            driver: "openai".to_string(),
            auth_style: "bearer".to_string(),
            priority: 0,
            enabled: true,
            remark: String::new(),
            has_key: false,
            key_mask: None,
            models,
        }
    }

    #[tokio::test]
    async fn migrates_legacy_ai_data_only_once() {
        let pool = legacy_pool().await;
        sqlx::query(
            "INSERT INTO provider_accounts(id, provider, model, enabled) VALUES ('legacy-model', 'legacy', 'legacy-remote', 1)",
        )
        .execute(&pool)
        .await
        .expect("seed legacy provider");
        sqlx::query(
            "INSERT INTO usage_ledger(provider_account_id, metric, amount, idempotency_key, created_at) VALUES ('legacy-model', 'requests', 2, 'legacy-usage', 10)",
        )
        .execute(&pool)
        .await
        .expect("seed legacy usage");

        initialize_ai_config_schema(&pool)
            .await
            .expect("run initial migration");
        assert_eq!(
            list_ai_providers(&pool)
                .await
                .expect("list providers")
                .len(),
            1
        );
        delete_ai_provider(&pool, "legacy")
            .await
            .expect("delete migrated provider");

        initialize_ai_config_schema(&pool)
            .await
            .expect("reinitialize schema");
        assert!(
            list_ai_providers(&pool)
                .await
                .expect("list providers after reinitialize")
                .is_empty()
        );
        let migration_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ai_config_migrations WHERE name = 'legacy-ai-config-v1'",
        )
        .fetch_one(&pool)
        .await
        .expect("read migration marker");
        assert_eq!(migration_count, 1);
    }

    #[tokio::test]
    async fn replaces_configuration_atomically_and_preserves_binding_order() {
        let pool = test_pool().await;
        upsert_ai_provider(&pool, &provider("old", Vec::new()))
            .await
            .expect("seed old provider");
        let new_provider = provider(
            "new",
            vec![
                model("model-a", "new", 0, "a"),
                model("model-b", "new", 0, "b"),
            ],
        );
        let bindings = HashMap::from([(
            "brief".to_string(),
            vec!["model-b".to_string(), "model-a".to_string()],
        )]);

        replace_ai_configuration(&pool, &[new_provider], &bindings)
            .await
            .expect("replace configuration");

        let providers = list_ai_providers(&pool).await.expect("list providers");
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].code, "new");
        assert_eq!(
            list_agent_bindings(&pool).await.expect("list bindings")["brief"],
            ["model-b", "model-a"]
        );
    }

    #[tokio::test]
    async fn rejects_invalid_replacement_without_deleting_current_configuration() {
        let pool = test_pool().await;
        upsert_ai_provider(&pool, &provider("current", Vec::new()))
            .await
            .expect("seed provider");
        let invalid = HashMap::from([("brief".to_string(), vec!["missing".to_string()])]);

        assert!(matches!(
            replace_ai_configuration(&pool, &[], &invalid).await,
            Err(StoreError::NotFound(_))
        ));
        assert_eq!(
            list_ai_providers(&pool).await.expect("list providers")[0].code,
            "current"
        );
    }

    #[tokio::test]
    async fn enforces_shared_limits_and_idempotent_reservations() {
        let pool = test_pool().await;
        upsert_ai_provider(&pool, &provider("shared", Vec::new()))
            .await
            .expect("seed provider");
        upsert_ai_model(&pool, &model("model-a", "shared", 2, "shared-group"))
            .await
            .expect("seed first model");
        upsert_ai_model(&pool, &model("model-b", "shared", 2, "shared-group"))
            .await
            .expect("seed second model");

        assert!(
            reserve_ai_usage(&pool, "model-a", "request-a", &[(LimitKind::Calls, 1)], 100)
                .await
                .expect("reserve first call")
        );
        assert!(
            !reserve_ai_usage(&pool, "model-a", "request-a", &[(LimitKind::Calls, 1)], 100)
                .await
                .expect("repeat reservation")
        );
        assert!(
            reserve_ai_usage(&pool, "model-b", "request-b", &[(LimitKind::Calls, 1)], 100)
                .await
                .expect("reserve shared call")
        );
        assert!(matches!(
            reserve_ai_usage(&pool, "model-a", "request-c", &[(LimitKind::Calls, 1)], 100).await,
            Err(StoreError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn opens_breaker_at_threshold_and_recovers_after_cooldown() {
        let pool = test_pool().await;
        let mut configured_model = model("breaker-model", "breaker", 0, "default");
        configured_model.params = json!({
            "breakerThreshold": 2,
            "breakerCooldownSeconds": 10
        });
        upsert_ai_provider(&pool, &provider("breaker", Vec::new()))
            .await
            .expect("seed provider");
        upsert_ai_model(&pool, &configured_model)
            .await
            .expect("seed model");
        write_agent_binding(&pool, "brief", &[configured_model.id.clone()])
            .await
            .expect("bind model");

        record_ai_route_failure(&pool, &configured_model.id, "temporary", 100)
            .await
            .expect("record first failure");
        assert!(
            load_ai_route_models(&pool, "brief", 101)
                .await
                .expect("load route after first failure")[0]
                .available
        );
        record_ai_route_failure(&pool, &configured_model.id, "temporary", 102)
            .await
            .expect("record second failure");
        assert!(
            !load_ai_route_models(&pool, "brief", 105)
                .await
                .expect("load open breaker")[0]
                .available
        );
        assert!(
            load_ai_route_models(&pool, "brief", 112)
                .await
                .expect("load half-open breaker")[0]
                .available
        );
        record_ai_route_success(&pool, &configured_model.id)
            .await
            .expect("clear breaker");
        assert!(
            read_breaker(&pool, &configured_model.id)
                .await
                .expect("read breaker")
                .is_none()
        );
    }
}
