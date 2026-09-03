use crate::StoreError;
use sqlx::Row;
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAccountMetadata {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRouteBinding {
    pub scope_key: String,
    pub provider_account_id: String,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioUsageEntry {
    pub metric: String,
    pub amount: u64,
    pub event_payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRouteEvent {
    pub sequence: u64,
    pub event_type: String,
    pub payload_json: String,
    pub created_at: i64,
}

pub async fn upsert_provider_accounts(
    studio: &SqlitePool,
    accounts: &[ProviderAccountMetadata],
) -> Result<(), StoreError> {
    let mut transaction = studio.begin().await?;
    for account in accounts {
        sqlx::query(
            "INSERT INTO provider_accounts(id, provider, model, enabled) VALUES (?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET provider = excluded.provider, model = excluded.model, enabled = excluded.enabled",
        )
        .bind(&account.id)
        .bind(&account.provider)
        .bind(&account.model)
        .bind(account.enabled)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

pub async fn list_provider_accounts(
    studio: &SqlitePool,
) -> Result<Vec<ProviderAccountMetadata>, StoreError> {
    sqlx::query("SELECT id, provider, model, enabled FROM provider_accounts ORDER BY rowid")
        .fetch_all(studio)
        .await?
        .into_iter()
        .map(|row| {
            Ok(ProviderAccountMetadata {
                id: row.try_get("id")?,
                provider: row.try_get("provider")?,
                model: row.try_get("model")?,
                enabled: row.try_get("enabled")?,
            })
        })
        .collect()
}

pub async fn load_route_binding(
    studio: &SqlitePool,
    scope_key: &str,
) -> Result<Option<StoredRouteBinding>, StoreError> {
    let row = sqlx::query(
        "SELECT rb.scope_key, rb.provider_account_id, pa.provider, rb.model FROM route_bindings rb JOIN provider_accounts pa ON pa.id = rb.provider_account_id WHERE rb.scope_key = ? AND pa.enabled = 1",
    )
    .bind(scope_key)
    .fetch_optional(studio)
    .await?;
    row.map(|row| {
        Ok(StoredRouteBinding {
            scope_key: row.try_get("scope_key")?,
            provider_account_id: row.try_get("provider_account_id")?,
            provider: row.try_get("provider")?,
            model: row.try_get("model")?,
        })
    })
    .transpose()
}

pub async fn record_route_selection(
    studio: &SqlitePool,
    binding: &StoredRouteBinding,
    event_type: &str,
    event_payload_json: &str,
    created_at: i64,
) -> Result<(), StoreError> {
    let mut transaction = studio.begin().await?;
    sqlx::query(
        "INSERT INTO route_bindings(scope_key, provider_account_id, model, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT(scope_key) DO UPDATE SET provider_account_id = excluded.provider_account_id, model = excluded.model, updated_at = excluded.updated_at",
    )
    .bind(&binding.scope_key)
    .bind(&binding.provider_account_id)
    .bind(&binding.model)
    .bind(created_at)
    .execute(&mut *transaction)
    .await?;
    insert_route_event(&mut transaction, event_type, event_payload_json, created_at).await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn record_usage(
    studio: &SqlitePool,
    provider_account_id: &str,
    idempotency_key: &str,
    entries: &[StudioUsageEntry],
    created_at: i64,
) -> Result<bool, StoreError> {
    let mut transaction = studio.begin().await?;
    let already_recorded: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM usage_ledger WHERE provider_account_id = ? AND idempotency_key = ?)",
    )
    .bind(provider_account_id)
    .bind(idempotency_key)
    .fetch_one(&mut *transaction)
    .await?;
    if already_recorded {
        transaction.rollback().await?;
        return Ok(false);
    }
    for entry in entries {
        sqlx::query(
            "INSERT INTO usage_ledger(provider_account_id, metric, amount, idempotency_key, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(provider_account_id)
        .bind(&entry.metric)
        .bind(entry.amount as i64)
        .bind(idempotency_key)
        .bind(created_at)
        .execute(&mut *transaction)
        .await?;
        insert_route_event(
            &mut transaction,
            "usage.updated",
            &entry.event_payload_json,
            created_at,
        )
        .await?;
    }
    transaction.commit().await?;
    Ok(true)
}

pub async fn list_route_events(studio: &SqlitePool) -> Result<Vec<StoredRouteEvent>, StoreError> {
    sqlx::query(
        "SELECT sequence, event_type, payload_json, created_at FROM route_events ORDER BY sequence",
    )
    .fetch_all(studio)
    .await?
    .into_iter()
    .map(|row| {
        Ok(StoredRouteEvent {
            sequence: row.try_get::<i64, _>("sequence")? as u64,
            event_type: row.try_get("event_type")?,
            payload_json: row.try_get("payload_json")?,
            created_at: row.try_get("created_at")?,
        })
    })
    .collect()
}

async fn insert_route_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    event_type: &str,
    payload_json: &str,
    created_at: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO route_events(event_type, payload_json, created_at) VALUES (?, ?, ?)")
        .bind(event_type)
        .bind(payload_json)
        .bind(created_at)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_studio_store;
    use tempfile::tempdir;

    #[tokio::test]
    async fn route_selection_and_usage_are_durable_and_idempotent() {
        let directory = tempdir().expect("tempdir");
        let studio = open_studio_store(directory.path()).await.expect("studio");
        let account = ProviderAccountMetadata {
            id: "account-a".to_string(),
            provider: "provider-a".to_string(),
            model: "model-a".to_string(),
            enabled: true,
        };
        upsert_provider_accounts(&studio, std::slice::from_ref(&account))
            .await
            .expect("accounts");
        let binding = StoredRouteBinding {
            scope_key: "conversation:c1".to_string(),
            provider_account_id: account.id,
            provider: account.provider,
            model: account.model,
        };
        record_route_selection(&studio, &binding, "route.selected", "{}", 1)
            .await
            .expect("selection");
        let usage = StudioUsageEntry {
            metric: "requests".to_string(),
            amount: 1,
            event_payload_json: "{}".to_string(),
        };
        assert!(
            record_usage(
                &studio,
                "account-a",
                "attempt-a",
                std::slice::from_ref(&usage),
                2
            )
            .await
            .expect("first usage")
        );
        assert!(
            !record_usage(&studio, "account-a", "attempt-a", &[usage], 3)
                .await
                .expect("duplicate usage")
        );

        assert_eq!(
            load_route_binding(&studio, "conversation:c1")
                .await
                .expect("load binding"),
            Some(binding)
        );
        assert_eq!(list_route_events(&studio).await.expect("events").len(), 2);
    }
}
