use super::GameAppServerAdapter;
use codex_game_app_server_protocol::*;
use codex_game_domain::AgentDefinition;
use codex_game_domain::AiCapability;
use codex_game_domain::AiProvider;
use codex_game_domain::LimitKind;
use codex_game_domain::LimitPolicy;
use codex_game_domain::ProviderModel;
use codex_game_domain::ProviderPreset;
use codex_game_domain::ProviderPresetModel;
use codex_game_store::clear_ai_breaker;
use codex_game_store::create_ai_provider_configuration;
use codex_game_store::delete_ai_model;
use codex_game_store::delete_ai_provider;
use codex_game_store::list_agent_bindings;
use codex_game_store::list_ai_providers;
use codex_game_store::open_studio_store;
use codex_game_store::read_ai_usage;
use codex_game_store::replace_ai_configuration;
use codex_game_store::reset_ai_usage;
use codex_game_store::upsert_ai_model;
use codex_game_store::upsert_ai_provider;
use codex_game_store::write_agent_binding;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiSecrets {
    provider_keys: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportedAiConfig {
    providers: Vec<AiProvider>,
    agent_bindings: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderPresetFile {
    schema_version: u32,
    presets: Vec<ProviderPreset>,
}

impl GameAppServerAdapter {
    pub async fn ai_provider_list(
        &self,
        _params: GameAiProviderListParams,
    ) -> Result<GameAiProviderListResponse, String> {
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        let mut providers = list_ai_providers(&pool)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        let secrets = self.read_secrets()?;
        for provider in &mut providers {
            if let Some(key) = secrets.provider_keys.get(&provider.code) {
                provider.has_key = true;
                provider.key_mask = Some(mask_key(key));
            }
        }
        Ok(GameAiProviderListResponse {
            providers: providers.into_iter().map(provider_dto).collect(),
        })
    }

    pub async fn ai_provider_create(
        &self,
        params: GameAiProviderCreateParams,
    ) -> Result<GameAiProvider, String> {
        let mut provider = provider_from_dto(params.provider)?;
        validate_provider(&provider)?;
        for model in &provider.models {
            validate_model(model)?;
        }
        let bindings = validate_create_bindings(&provider, params.agent_bindings)?;
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        create_ai_provider_configuration(&pool, &provider, &bindings)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        let secrets = self.update_provider_secret(&provider.code, params.api_key)?;
        apply_secret_metadata(&mut provider, &secrets);
        self.sync_codex_provider_config().await?;
        Ok(provider_dto(provider))
    }

    pub async fn ai_provider_update(
        &self,
        params: GameAiProviderWriteParams,
    ) -> Result<GameAiProvider, String> {
        let mut provider = provider_from_dto(params.provider)?;
        validate_provider(&provider)?;
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        let existing = list_ai_providers(&pool)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|existing| existing.code == provider.code);
        let Some(existing) = existing else {
            pool.close().await;
            return Err(format!("AI provider {} does not exist", provider.code));
        };
        provider.models = existing.models;
        upsert_ai_provider(&pool, &provider)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        let secrets = self.update_provider_secret(&provider.code, params.api_key)?;
        apply_secret_metadata(&mut provider, &secrets);
        self.sync_codex_provider_config().await?;
        Ok(provider_dto(provider))
    }

    pub async fn ai_provider_delete(
        &self,
        params: GameAiProviderDeleteParams,
    ) -> Result<GameAiProviderDeleteResponse, String> {
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        delete_ai_provider(&pool, &params.code)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        let mut secrets = self.read_secrets()?;
        if secrets.provider_keys.remove(&params.code).is_some() {
            self.write_secrets(&secrets)?;
        }
        self.sync_codex_provider_config().await?;
        Ok(GameAiProviderDeleteResponse {})
    }

    pub async fn ai_model_write(
        &self,
        params: GameAiModelWriteParams,
    ) -> Result<GameAiModel, String> {
        let model = model_from_dto(params.model)?;
        validate_model(&model)?;
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        upsert_ai_model(&pool, &model)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        self.sync_codex_provider_config().await?;
        Ok(model_dto(model))
    }

    pub async fn ai_model_delete(
        &self,
        params: GameAiModelDeleteParams,
    ) -> Result<GameAiModelDeleteResponse, String> {
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        delete_ai_model(&pool, &params.model_id)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        self.sync_codex_provider_config().await?;
        Ok(GameAiModelDeleteResponse {})
    }

    pub async fn ai_agent_list(
        &self,
        _params: GameAiAgentListParams,
    ) -> Result<GameAiAgentListResponse, String> {
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        let bindings = list_agent_bindings(&pool)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        let agents = agent_definitions()
            .into_iter()
            .map(|mut agent| {
                agent.model_ids = bindings.get(&agent.agent_code).cloned().unwrap_or_default();
                agent_dto(agent)
            })
            .collect();
        Ok(GameAiAgentListResponse { agents })
    }

    pub async fn ai_agent_binding_write(
        &self,
        params: GameAiAgentBindingWriteParams,
    ) -> Result<GameAiAgentBindingWriteResponse, String> {
        if !agent_definitions()
            .iter()
            .any(|agent| agent.agent_code == params.agent_code)
        {
            return Err(format!("unknown agent: {}", params.agent_code));
        }
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        write_agent_binding(&pool, &params.agent_code, &params.model_ids)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        Ok(GameAiAgentBindingWriteResponse {})
    }

    pub async fn ai_usage_read(
        &self,
        _params: GameAiUsageReadParams,
    ) -> Result<GameAiUsageReadResponse, String> {
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        let mut items = read_ai_usage(&pool)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        let secrets = self.read_secrets()?;
        for item in &mut items {
            item.has_key = secrets.provider_keys.contains_key(&item.provider_code);
        }
        Ok(GameAiUsageReadResponse {
            items: items.into_iter().map(usage_dto).collect(),
        })
    }

    pub async fn ai_usage_reset(
        &self,
        params: GameAiUsageResetParams,
    ) -> Result<GameAiUsageResetResponse, String> {
        let limit_kind = params
            .limit_kind
            .as_deref()
            .map(parse_limit_kind)
            .transpose()?;
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        let cleared = reset_ai_usage(&pool, &params.model_id, limit_kind.as_ref())
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        Ok(GameAiUsageResetResponse { cleared })
    }

    pub async fn ai_breaker_clear(
        &self,
        params: GameAiBreakerClearParams,
    ) -> Result<GameAiBreakerClearResponse, String> {
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        clear_ai_breaker(&pool, &params.model_id)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        Ok(GameAiBreakerClearResponse {})
    }

    pub fn provider_preset_list(
        &self,
        _params: GameProviderPresetListParams,
    ) -> Result<GameProviderPresetListResponse, String> {
        let path = self.recommendation_path()?;
        let document = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let file: ProviderPresetFile =
            toml::from_str(&document).map_err(|error| error.to_string())?;
        if file.schema_version != 2 {
            return Err(format!(
                "unsupported provider preset schema: {}",
                file.schema_version
            ));
        }
        validate_presets(&file.presets)?;
        Ok(GameProviderPresetListResponse {
            presets: file.presets.into_iter().map(preset_dto).collect(),
            path: path.to_string_lossy().into_owned(),
        })
    }

    pub async fn ai_config_export(
        &self,
        _params: GameAiConfigExportParams,
    ) -> Result<GameAiConfigExportResponse, String> {
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        let mut providers = list_ai_providers(&pool)
            .await
            .map_err(|error| error.to_string())?;
        let bindings = list_agent_bindings(&pool)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        for provider in &mut providers {
            provider.has_key = false;
            provider.key_mask = None;
        }
        let bundle = ExportedAiConfig {
            providers,
            agent_bindings: bindings.into_iter().collect(),
        };
        serde_json::to_string_pretty(&bundle)
            .map(|json| GameAiConfigExportResponse { json })
            .map_err(|error| error.to_string())
    }

    pub async fn ai_config_import(
        &self,
        params: GameAiConfigImportParams,
    ) -> Result<GameAiConfigImportResponse, String> {
        let bundle: ExportedAiConfig =
            serde_json::from_str(&params.json).map_err(|error| error.to_string())?;
        validate_import_bundle(&bundle)?;
        let provider_count = bundle.providers.len() as u64;
        let model_count = bundle
            .providers
            .iter()
            .map(|provider| provider.models.len() as u64)
            .sum();
        if !params.dry_run {
            let pool = open_studio_store(&self.studio_storage)
                .await
                .map_err(|error| error.to_string())?;
            let provider_codes = bundle
                .providers
                .iter()
                .map(|provider| provider.code.clone())
                .collect::<HashSet<_>>();
            let bindings = bundle.agent_bindings.into_iter().collect::<HashMap<_, _>>();
            replace_ai_configuration(&pool, &bundle.providers, &bindings)
                .await
                .map_err(|error| error.to_string())?;
            pool.close().await;
            let mut secrets = self.read_secrets()?;
            let previous_secret_count = secrets.provider_keys.len();
            secrets
                .provider_keys
                .retain(|provider_code, _| provider_codes.contains(provider_code));
            if secrets.provider_keys.len() != previous_secret_count {
                self.write_secrets(&secrets)?;
            }
            self.sync_codex_provider_config().await?;
        }
        Ok(GameAiConfigImportResponse {
            provider_count,
            model_count,
            applied: !params.dry_run,
        })
    }

    async fn sync_codex_provider_config(&self) -> Result<(), String> {
        let pool = open_studio_store(&self.studio_storage)
            .await
            .map_err(|error| error.to_string())?;
        let providers = list_ai_providers(&pool)
            .await
            .map_err(|error| error.to_string())?;
        pool.close().await;
        fs::create_dir_all(&self.studio_storage).map_err(|error| error.to_string())?;
        let config_path = self.studio_storage.join("config.toml");
        let mut config = match fs::read_to_string(&config_path) {
            Ok(document) => document
                .parse::<toml::Table>()
                .map_err(|error| format!("invalid project-local config.toml: {error}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
            Err(error) => return Err(error.to_string()),
        };
        let managed_path = self
            .studio_storage
            .parent()
            .ok_or_else(|| "invalid project-local Codex home".to_string())?
            .join("codex-provider-codes.json");
        let previous = fs::read_to_string(&managed_path)
            .ok()
            .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
            .unwrap_or_default();
        let model_providers = config
            .entry("model_providers")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| "config.toml model_providers must be a table".to_string())?;
        for code in previous {
            model_providers.remove(&code);
        }
        let mut managed = Vec::with_capacity(providers.len());
        for provider in providers {
            let Some(base_url) = responses_base_url(&provider) else {
                continue;
            };
            managed.push(provider.code.clone());
            let mut entry = toml::Table::new();
            entry.insert("name".to_string(), toml::Value::String(provider.name));
            if !base_url.is_empty() {
                entry.insert("base_url".to_string(), toml::Value::String(base_url));
            }
            let environment = provider_key_environment(&provider.code);
            match provider.auth_style.as_str() {
                "bearer" => {
                    entry.insert("env_key".to_string(), toml::Value::String(environment));
                }
                "x-api-key" => {
                    let mut headers = toml::Table::new();
                    headers.insert("x-api-key".to_string(), toml::Value::String(environment));
                    entry.insert("env_http_headers".to_string(), toml::Value::Table(headers));
                }
                _ => continue,
            }
            entry.insert(
                "wire_api".to_string(),
                toml::Value::String("responses".to_string()),
            );
            entry.insert(
                "requires_openai_auth".to_string(),
                toml::Value::Boolean(false),
            );
            model_providers.insert(provider.code, toml::Value::Table(entry));
        }
        let temporary = config_path.with_extension("toml.tmp");
        fs::write(
            &temporary,
            toml::to_string_pretty(&config).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        fs::rename(temporary, config_path).map_err(|error| error.to_string())?;
        fs::write(
            managed_path,
            serde_json::to_vec_pretty(&managed).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn secret_path(&self) -> Result<PathBuf, String> {
        self.studio_storage
            .parent()
            .map(|path| path.join("ai-secrets.json"))
            .ok_or_else(|| "invalid project-local Codex home".to_string())
    }

    fn recommendation_path(&self) -> Result<PathBuf, String> {
        self.studio_storage
            .parent()
            .and_then(Path::parent)
            .map(|path| path.join("model-recommendations.toml"))
            .ok_or_else(|| "invalid project-local Codex home".to_string())
    }

    fn update_provider_secret(
        &self,
        provider_code: &str,
        api_key: Option<String>,
    ) -> Result<AiSecrets, String> {
        let mut secrets = self.read_secrets()?;
        if let Some(api_key) = api_key {
            if api_key.trim().is_empty() {
                secrets.provider_keys.remove(provider_code);
            } else {
                secrets
                    .provider_keys
                    .insert(provider_code.to_string(), api_key);
            }
            self.write_secrets(&secrets)?;
        }
        Ok(secrets)
    }

    fn read_secrets(&self) -> Result<AiSecrets, String> {
        let path = self.secret_path()?;
        match fs::read_to_string(path) {
            Ok(value) => serde_json::from_str(&value).map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AiSecrets::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write_secrets(&self, secrets: &AiSecrets) -> Result<(), String> {
        let path = self.secret_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(
            &path,
            serde_json::to_vec_pretty(secrets).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        restrict_secret_permissions(&path).map_err(|error| error.to_string())
    }
}

fn apply_secret_metadata(provider: &mut AiProvider, secrets: &AiSecrets) {
    provider.has_key = false;
    provider.key_mask = None;
    if let Some(key) = secrets.provider_keys.get(&provider.code) {
        provider.has_key = true;
        provider.key_mask = Some(mask_key(key));
    }
}

fn validate_create_bindings(
    provider: &AiProvider,
    bindings: Vec<GameAiAgentBinding>,
) -> Result<HashMap<String, Vec<String>>, String> {
    let known_agents = agent_definitions()
        .into_iter()
        .map(|agent| agent.agent_code)
        .collect::<HashSet<_>>();
    let model_ids = provider
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<HashSet<_>>();
    let mut result = HashMap::new();
    for binding in bindings {
        if !known_agents.contains(&binding.agent_code) {
            return Err(format!("unknown agent: {}", binding.agent_code));
        }
        if result.contains_key(&binding.agent_code) {
            return Err(format!("duplicate agent binding: {}", binding.agent_code));
        }
        let mut unique_model_ids = HashSet::new();
        for model_id in &binding.model_ids {
            if !model_ids.contains(model_id.as_str()) {
                return Err(format!("unknown AI model in binding: {model_id}"));
            }
            if !unique_model_ids.insert(model_id) {
                return Err(format!(
                    "duplicate AI model {model_id} in agent binding {}",
                    binding.agent_code
                ));
            }
        }
        result.insert(binding.agent_code, binding.model_ids);
    }
    Ok(result)
}

fn validate_presets(presets: &[ProviderPreset]) -> Result<(), String> {
    let mut preset_codes = HashSet::new();
    for preset in presets {
        if preset.code.trim().is_empty()
            || preset.vendor.trim().is_empty()
            || preset.plan.trim().is_empty()
            || preset.label.trim().is_empty()
            || preset.base_url.trim().is_empty()
            || preset.driver.trim().is_empty()
        {
            return Err("provider preset metadata must not be empty".to_string());
        }
        if !preset_codes.insert(preset.code.as_str()) {
            return Err(format!("duplicate provider preset code: {}", preset.code));
        }
        if !matches!(preset.auth_style.as_str(), "bearer" | "x-api-key") {
            return Err(format!(
                "unsupported provider preset auth style: {}",
                preset.auth_style
            ));
        }
        if preset.models.is_empty() {
            return Err(format!("provider preset {} has no models", preset.code));
        }
        let mut model_ids = HashSet::new();
        for model in &preset.models {
            if model.model_id.trim().is_empty()
                || model.driver.trim().is_empty()
                || model.api_path.trim().is_empty()
                || model.default_period.trim().is_empty()
                || model.capabilities.is_empty()
                || !model.params.is_object()
            {
                return Err(format!(
                    "provider preset {} contains an invalid model",
                    preset.code
                ));
            }
            if !model_ids.insert(model.model_id.as_str()) {
                return Err(format!(
                    "duplicate model {} in provider preset {}",
                    model.model_id, preset.code
                ));
            }
        }
    }
    Ok(())
}

fn responses_base_url(provider: &AiProvider) -> Option<String> {
    let model = provider.models.iter().find(|model| {
        model.enabled && matches!(model.driver.as_str(), "openai" | "openai_compat")
    })?;
    let base = provider.base_url.trim_end_matches('/');
    let mut api_path = model.api_path.trim().trim_end_matches('/');
    if let Some(prefix) = api_path.strip_suffix("/responses") {
        api_path = prefix;
    }
    if api_path.is_empty() {
        Some(base.to_string())
    } else if base.is_empty() {
        Some(api_path.to_string())
    } else {
        Some(format!("{base}/{}", api_path.trim_start_matches('/')))
    }
}

fn validate_import_bundle(bundle: &ExportedAiConfig) -> Result<(), String> {
    let known_agents = agent_definitions()
        .into_iter()
        .map(|agent| agent.agent_code)
        .collect::<HashSet<_>>();
    let mut provider_codes = HashSet::new();
    let mut model_ids = HashSet::new();
    for provider in &bundle.providers {
        validate_provider(provider)?;
        if !provider_codes.insert(provider.code.as_str()) {
            return Err(format!("duplicate provider code: {}", provider.code));
        }
        let mut provider_model_ids = HashSet::new();
        for model in &provider.models {
            validate_model(model)?;
            if !provider_model_ids.insert(model.model_id.as_str()) {
                return Err(format!(
                    "duplicate model {} in provider {}",
                    model.model_id, provider.code
                ));
            }
            if model.provider_code != provider.code {
                return Err(format!(
                    "model {} belongs to provider {}, expected {}",
                    model.id, model.provider_code, provider.code
                ));
            }
            if !model_ids.insert(model.id.as_str()) {
                return Err(format!("duplicate model id: {}", model.id));
            }
        }
    }
    for (agent_code, binding_model_ids) in &bundle.agent_bindings {
        if !known_agents.contains(agent_code) {
            return Err(format!("unknown agent: {agent_code}"));
        }
        let mut unique_binding_ids = HashSet::new();
        for model_id in binding_model_ids {
            if !model_ids.contains(model_id.as_str()) {
                return Err(format!("unknown AI model in binding: {model_id}"));
            }
            if !unique_binding_ids.insert(model_id) {
                return Err(format!(
                    "duplicate AI model {model_id} in agent binding {agent_code}"
                ));
            }
        }
    }
    Ok(())
}

fn validate_provider(provider: &AiProvider) -> Result<(), String> {
    if provider.code.is_empty()
        || !provider
            .code
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
    {
        return Err("provider code must contain only letters, numbers, '-' or '_'".to_string());
    }
    if provider.name.trim().is_empty() || provider.driver.trim().is_empty() {
        return Err("provider name and driver must not be empty".to_string());
    }
    if !matches!(provider.auth_style.as_str(), "bearer" | "x-api-key") {
        return Err(format!(
            "unsupported provider auth style: {}",
            provider.auth_style
        ));
    }
    Ok(())
}

fn validate_model(model: &ProviderModel) -> Result<(), String> {
    if model.id.trim().is_empty() || model.model_id.trim().is_empty() {
        return Err("model id must not be empty".to_string());
    }
    let mut kinds = std::collections::HashSet::new();
    for limit in &model.limits {
        if !kinds.insert(limit_kind_name(&limit.limit_kind)) {
            return Err("each limit kind can only be configured once".to_string());
        }
        if limit.period_expr.trim().is_empty() || limit.group_name.trim().is_empty() {
            return Err("limit period and group must not be empty".to_string());
        }
    }
    Ok(())
}

fn provider_from_dto(provider: GameAiProvider) -> Result<AiProvider, String> {
    Ok(AiProvider {
        code: provider.code,
        name: provider.name,
        base_url: provider.base_url,
        driver: provider.driver,
        auth_style: provider.auth_style,
        priority: provider.priority,
        enabled: provider.enabled,
        remark: provider.remark,
        has_key: provider.has_key,
        key_mask: provider.key_mask,
        models: provider
            .models
            .into_iter()
            .map(model_from_dto)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn model_from_dto(model: GameAiModel) -> Result<ProviderModel, String> {
    Ok(ProviderModel {
        id: model.id,
        provider_code: model.provider_code,
        model_id: model.model_id,
        display_name: model.display_name,
        capabilities: model
            .capabilities
            .iter()
            .map(|value| parse_capability(value))
            .collect::<Result<Vec<_>, _>>()?,
        driver: model.driver,
        api_path: model.api_path,
        enabled: model.enabled,
        sort_no: model.sort_no,
        params: serde_json::from_str(&model.params_json).map_err(|error| error.to_string())?,
        remark: model.remark,
        limits: model
            .limits
            .into_iter()
            .map(|limit| {
                Ok(LimitPolicy {
                    limit_kind: parse_limit_kind(&limit.limit_kind)?,
                    max_value: limit.max_value,
                    period_expr: limit.period_expr,
                    group_name: limit.group_name,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    })
}

fn provider_dto(provider: AiProvider) -> GameAiProvider {
    GameAiProvider {
        code: provider.code,
        name: provider.name,
        base_url: provider.base_url,
        driver: provider.driver,
        auth_style: provider.auth_style,
        priority: provider.priority,
        enabled: provider.enabled,
        remark: provider.remark,
        has_key: provider.has_key,
        key_mask: provider.key_mask,
        models: provider.models.into_iter().map(model_dto).collect(),
    }
}

fn model_dto(model: ProviderModel) -> GameAiModel {
    GameAiModel {
        id: model.id,
        provider_code: model.provider_code,
        model_id: model.model_id,
        display_name: model.display_name,
        capabilities: model.capabilities.iter().map(capability_name).collect(),
        driver: model.driver,
        api_path: model.api_path,
        enabled: model.enabled,
        sort_no: model.sort_no,
        params_json: model.params.to_string(),
        remark: model.remark,
        limits: model.limits.into_iter().map(limit_dto).collect(),
    }
}

fn agent_dto(agent: AgentDefinition) -> GameAiAgent {
    GameAiAgent {
        agent_code: agent.agent_code,
        role: agent.role,
        capability: capability_name(&agent.capability),
        output_contract: agent.output_contract,
        source_file: agent.source_file,
        model_ids: agent.model_ids,
    }
}

fn usage_dto(usage: codex_game_domain::ModelUsage) -> GameAiModelUsage {
    GameAiModelUsage {
        provider_code: usage.provider_code,
        provider_name: usage.provider_name,
        provider_model_id: usage.provider_model_id,
        model_id: usage.model_id,
        provider_enabled: usage.provider_enabled,
        enabled: usage.enabled,
        has_key: usage.has_key,
        agents: usage.agents,
        budgets: usage
            .budgets
            .into_iter()
            .map(|budget| GameAiUsageBudget {
                limit_kind: limit_kind_name(&budget.limit_kind),
                used: budget.used,
                limit: budget.limit,
                period_expr: budget.period_expr,
                window_key: budget.window_key,
                group_name: budget.group_name,
                source: budget.source,
                exhausted: budget.exhausted,
                unlimited: budget.unlimited,
            })
            .collect(),
        breaker: usage.breaker.map(|breaker| GameAiBreaker {
            failure_count: breaker.failure_count,
            last_reason: breaker.last_reason,
            opened_at: breaker.opened_at,
            retry_at: breaker.retry_at,
        }),
    }
}

fn preset_dto(preset: ProviderPreset) -> GameProviderPreset {
    GameProviderPreset {
        code: preset.code,
        vendor: preset.vendor,
        plan: preset.plan,
        label: preset.label,
        base_url: preset.base_url,
        driver: preset.driver,
        auth_style: preset.auth_style,
        key_prefix: preset.key_prefix,
        models: preset.models.into_iter().map(preset_model_dto).collect(),
    }
}

fn preset_model_dto(model: ProviderPresetModel) -> GameProviderPresetModel {
    GameProviderPresetModel {
        model_id: model.model_id,
        capabilities: model.capabilities.iter().map(capability_name).collect(),
        driver: model.driver,
        api_path: model.api_path,
        limit_kind: limit_kind_name(&model.limit_kind),
        default_period: model.default_period,
        params_json: model.params.to_string(),
        remark: model.remark,
    }
}

fn limit_dto(limit: LimitPolicy) -> GameAiLimit {
    GameAiLimit {
        limit_kind: limit_kind_name(&limit.limit_kind),
        max_value: limit.max_value,
        period_expr: limit.period_expr,
        group_name: limit.group_name,
    }
}

fn parse_capability(value: &str) -> Result<AiCapability, String> {
    match value {
        "text_reasoning" => Ok(AiCapability::TextReasoning),
        "text_structured_output" => Ok(AiCapability::TextStructuredOutput),
        "vision_analysis" => Ok(AiCapability::VisionAnalysis),
        "image_text_to_image" => Ok(AiCapability::ImageTextToImage),
        "image_image_to_image" => Ok(AiCapability::ImageImageToImage),
        "image_reference_consistency" => Ok(AiCapability::ImageReferenceConsistency),
        "video_text_to_video" => Ok(AiCapability::VideoTextToVideo),
        "video_image_to_video" => Ok(AiCapability::VideoImageToVideo),
        "model3d" => Ok(AiCapability::Model3d),
        other => Err(format!("unknown AI capability: {other}")),
    }
}

fn capability_name(capability: &AiCapability) -> String {
    match capability {
        AiCapability::TextReasoning => "text_reasoning",
        AiCapability::TextStructuredOutput => "text_structured_output",
        AiCapability::VisionAnalysis => "vision_analysis",
        AiCapability::ImageTextToImage => "image_text_to_image",
        AiCapability::ImageImageToImage => "image_image_to_image",
        AiCapability::ImageReferenceConsistency => "image_reference_consistency",
        AiCapability::VideoTextToVideo => "video_text_to_video",
        AiCapability::VideoImageToVideo => "video_image_to_video",
        AiCapability::Model3d => "model3d",
    }
    .to_string()
}

fn parse_limit_kind(value: &str) -> Result<LimitKind, String> {
    match value {
        "calls" => Ok(LimitKind::Calls),
        "input_tokens" => Ok(LimitKind::InputTokens),
        "output_tokens" => Ok(LimitKind::OutputTokens),
        "total_tokens" => Ok(LimitKind::TotalTokens),
        "tokens" => Ok(LimitKind::Tokens),
        "credits" => Ok(LimitKind::Credits),
        other => Err(format!("unknown limit kind: {other}")),
    }
}

fn limit_kind_name(kind: &LimitKind) -> String {
    match kind {
        LimitKind::Calls => "calls",
        LimitKind::InputTokens => "input_tokens",
        LimitKind::OutputTokens => "output_tokens",
        LimitKind::TotalTokens => "total_tokens",
        LimitKind::Tokens => "tokens",
        LimitKind::Credits => "credits",
    }
    .to_string()
}

fn provider_key_environment(code: &str) -> String {
    format!(
        "CODEX_GAME_PROVIDER_{}_API_KEY",
        code.chars()
            .map(|character| if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            })
            .collect::<String>()
    )
}

fn mask_key(key: &str) -> String {
    let suffix = key
        .char_indices()
        .rev()
        .nth(3)
        .map_or(key, |(index, _)| &key[index..]);
    format!("••••{suffix}")
}

fn agent_definitions() -> Vec<AgentDefinition> {
    [
        ("brief", "需求分析", AiCapability::TextReasoning),
        (
            "game-design-review",
            "玩法设计评审",
            AiCapability::TextStructuredOutput,
        ),
        (
            "visual-style-review",
            "视觉风格评审",
            AiCapability::VisionAnalysis,
        ),
        (
            "production-feasibility-review",
            "制作可行性评审",
            AiCapability::TextReasoning,
        ),
        ("synthesis", "方案综合", AiCapability::TextStructuredOutput),
    ]
    .into_iter()
    .map(|(code, role, capability)| AgentDefinition {
        agent_code: code.to_string(),
        role: role.to_string(),
        capability,
        output_contract: "json_schema".to_string(),
        source_file: format!("game/runtime/agents/{code}.md"),
        model_ids: Vec::new(),
    })
    .collect()
}

#[cfg(unix)]
fn restrict_secret_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_secret_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn provider_dto_for_test() -> GameAiProvider {
        GameAiProvider {
            code: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            base_url: "https://example.test/v1".to_string(),
            driver: "openai".to_string(),
            auth_style: "bearer".to_string(),
            priority: 0,
            enabled: true,
            remark: String::new(),
            has_key: false,
            key_mask: None,
            models: Vec::new(),
        }
    }

    #[tokio::test]
    async fn provider_api_never_returns_plaintext_keys() {
        let directory = tempdir().expect("tempdir");
        let codex_home = directory.path().join(".codex-game/local/codex-home");
        let adapter = GameAppServerAdapter::new(codex_home.clone());
        let secret = "secret-value-1234";

        let created = adapter
            .ai_provider_create(GameAiProviderCreateParams {
                provider: provider_dto_for_test(),
                api_key: Some(secret.to_string()),
                agent_bindings: Vec::new(),
            })
            .await
            .expect("create provider");
        assert!(created.has_key);
        assert_eq!(created.key_mask.as_deref(), Some("••••1234"));
        assert!(
            !serde_json::to_string(&created)
                .expect("serialize response")
                .contains(secret)
        );

        let listed = adapter
            .ai_provider_list(GameAiProviderListParams {})
            .await
            .expect("list providers");
        assert!(
            !serde_json::to_string(&listed)
                .expect("serialize list")
                .contains(secret)
        );
        let exported = adapter
            .ai_config_export(GameAiConfigExportParams {})
            .await
            .expect("export configuration");
        assert!(!exported.json.contains(secret));

        let imported = adapter
            .ai_config_import(GameAiConfigImportParams {
                json: r#"{"providers":[],"agentBindings":{}}"#.to_string(),
                dry_run: false,
            })
            .await
            .expect("replace configuration");
        assert!(imported.applied);
        assert!(
            adapter
                .ai_provider_list(GameAiProviderListParams {})
                .await
                .expect("list providers after import")
                .providers
                .is_empty()
        );
        assert!(
            !fs::read_to_string(
                codex_home
                    .parent()
                    .expect("local directory")
                    .join("ai-secrets.json"),
            )
            .expect("read secrets")
            .contains(secret)
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(
                codex_home
                    .parent()
                    .expect("local directory")
                    .join("ai-secrets.json"),
            )
            .expect("read secret metadata")
            .permissions()
            .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn provider_preset_schema_rejects_secret_fields() {
        let document = r#"
            schema_version = 2

            [[presets]]
            code = "unsafe"
            vendor = "Unsafe"
            plan = "Unsafe"
            label = "Unsafe"
            base_url = "https://example.test"
            driver = "openai_compat"
            auth_style = "bearer"
            api_key = "must-not-be-accepted"
            models = []
        "#;

        assert!(toml::from_str::<ProviderPresetFile>(document).is_err());
    }
}
