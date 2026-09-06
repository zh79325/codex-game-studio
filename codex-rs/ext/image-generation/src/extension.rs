use std::sync::Arc;

use codex_core::config::Config;
use codex_extension_api::ConfigContributor;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionFuture;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_extension_api::ThreadLifecycleContributor;
use codex_extension_api::ThreadOriginator;
use codex_extension_api::ThreadStartInput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolContributor;
use codex_extension_api::ToolExecutor;
use codex_login::AuthManager;
use codex_model_provider::create_model_provider;
use codex_model_provider_info::ModelProviderInfo;
use codex_utils_absolute_path::AbsolutePathBuf;

use crate::backend::CodexImagesBackend;
use crate::dialect::ImageApiDialect;
use crate::tool::ImageGenerationTool;
use crate::tool::ImageGenerationTurnGate;

#[derive(Clone)]
struct ImageGenerationExtension {
    auth_manager: Arc<AuthManager>,
    resolve_save_root: Arc<SaveRootResolver>,
}

type SaveRootResolver = dyn Fn(&Config) -> Option<AbsolutePathBuf> + Send + Sync;

/// Host-provided image routes for threads whose chat model differs from image executors.
#[derive(Clone, Debug)]
pub struct ImageGenerationRouteOverride {
    pub tools: Vec<ImageGenerationToolRouteOverride>,
}

#[derive(Clone, Debug)]
pub struct ImageGenerationToolRouteOverride {
    pub provider: ModelProviderInfo,
    pub model: String,
    pub api_dialect: ImageApiDialect,
    pub save_root: Option<AbsolutePathBuf>,
    pub tool_name: String,
}

#[derive(Clone)]
struct ImageGenerationExtensionConfig {
    tools: Vec<ImageGenerationToolConfig>,
}

#[derive(Clone)]
struct ImageGenerationToolConfig {
    provider: ModelProviderInfo,
    model: String,
    api_dialect: ImageApiDialect,
    save_root: Option<AbsolutePathBuf>,
    tool_name: Option<String>,
}

impl ImageGenerationExtensionConfig {
    /// Resolves the image provider, model, and save root for a thread.
    fn from_config(
        config: &Config,
        resolve_save_root: &SaveRootResolver,
        route_override: Option<&ImageGenerationRouteOverride>,
    ) -> Self {
        if let Some(route_override) = route_override {
            return Self {
                tools: route_override
                    .tools
                    .iter()
                    .map(|tool| ImageGenerationToolConfig {
                        provider: tool.provider.clone(),
                        model: tool.model.clone(),
                        api_dialect: tool.api_dialect,
                        save_root: tool.save_root.clone(),
                        tool_name: Some(tool.tool_name.clone()),
                    })
                    .collect(),
            };
        }
        let available = config.model_provider.is_openai()
            || config.model_provider.requires_openai_auth
            || config.model_provider.uses_openai_actor_authorization();
        Self {
            tools: available
                .then(|| ImageGenerationToolConfig {
                    provider: config.model_provider.clone(),
                    model: "gpt-image-2".to_string(),
                    api_dialect: ImageApiDialect::OpenAi,
                    save_root: resolve_save_root(config),
                    tool_name: None,
                })
                .into_iter()
                .collect(),
        }
    }
}

impl ThreadLifecycleContributor<Config> for ImageGenerationExtension {
    /// Seeds image-generation configuration when a thread begins.
    fn on_thread_start<'a>(
        &'a self,
        input: ThreadStartInput<'a, Config>,
    ) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            input
                .thread_store
                .insert(ImageGenerationTurnGate::default());
            input
                .thread_store
                .insert(ImageGenerationExtensionConfig::from_config(
                    input.config,
                    self.resolve_save_root.as_ref(),
                    input
                        .thread_store
                        .get::<ImageGenerationRouteOverride>()
                        .as_deref(),
                ));
        })
    }
}

impl ConfigContributor<Config> for ImageGenerationExtension {
    /// Refreshes image-generation configuration after thread configuration changes.
    fn on_config_changed(
        &self,
        _session_store: &ExtensionData,
        thread_store: &ExtensionData,
        _previous_config: &Config,
        new_config: &Config,
    ) {
        thread_store.insert(ImageGenerationExtensionConfig::from_config(
            new_config,
            self.resolve_save_root.as_ref(),
            thread_store
                .get::<ImageGenerationRouteOverride>()
                .as_deref(),
        ));
    }
}

impl ToolContributor for ImageGenerationExtension {
    /// Creates the image-generation tool exposed by this installed extension.
    fn tools(
        &self,
        _session_store: &ExtensionData,
        thread_store: &ExtensionData,
    ) -> Vec<Arc<dyn for<'call> ToolExecutor<ToolCall<'call>>>> {
        let Some(config) = thread_store.get::<ImageGenerationExtensionConfig>() else {
            return Vec::new();
        };
        let originator = thread_store
            .get::<ThreadOriginator>()
            .map(|originator| originator.0.clone());
        let turn_gate = thread_store
            .get::<ImageGenerationTurnGate>()
            .unwrap_or_else(|| Arc::new(ImageGenerationTurnGate::default()));
        config
            .tools
            .iter()
            .map(|tool| {
                Arc::new(ImageGenerationTool::new(
                    CodexImagesBackend::new(
                        create_model_provider(
                            tool.provider.clone(),
                            Some(self.auth_manager.clone()),
                        ),
                        originator.clone(),
                        tool.api_dialect,
                    ),
                    tool.save_root.clone(),
                    thread_store.level_id().to_string(),
                    tool.model.clone(),
                    tool.tool_name.clone(),
                    turn_gate.clone(),
                )) as Arc<dyn for<'call> ToolExecutor<ToolCall<'call>>>
            })
            .collect()
    }
}

/// Installs the standalone image-generation extension contributors.
pub fn install(
    registry: &mut ExtensionRegistryBuilder<Config>,
    auth_manager: Arc<AuthManager>,
    resolve_save_root: impl Fn(&Config) -> Option<AbsolutePathBuf> + Send + Sync + 'static,
) {
    let extension = Arc::new(ImageGenerationExtension {
        auth_manager,
        resolve_save_root: Arc::new(resolve_save_root),
    });
    registry.thread_lifecycle_contributor(extension.clone());
    registry.config_contributor(extension.clone());
    registry.tool_contributor(extension);
}
