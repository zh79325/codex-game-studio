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
use crate::tool::ImageGenerationTool;

#[derive(Clone)]
struct ImageGenerationExtension {
    auth_manager: Arc<AuthManager>,
    resolve_save_root: Arc<SaveRootResolver>,
}

type SaveRootResolver = dyn Fn(&Config) -> Option<AbsolutePathBuf> + Send + Sync;

/// Host-provided image route for threads whose chat model differs from the image executor.
#[derive(Clone, Debug)]
pub struct ImageGenerationRouteOverride {
    pub provider: ModelProviderInfo,
    pub model: String,
    pub save_root: Option<AbsolutePathBuf>,
}

#[derive(Clone)]
struct ImageGenerationExtensionConfig {
    available: bool,
    provider: ModelProviderInfo,
    model: String,
    save_root: Option<AbsolutePathBuf>,
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
                available: true,
                provider: route_override.provider.clone(),
                model: route_override.model.clone(),
                save_root: route_override.save_root.clone(),
            };
        }
        Self {
            available: config.model_provider.is_openai()
                || config.model_provider.requires_openai_auth
                || config.model_provider.uses_openai_actor_authorization(),
            provider: config.model_provider.clone(),
            model: "gpt-image-2".to_string(),
            save_root: resolve_save_root(config),
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
        if !config.available {
            return Vec::new();
        }

        vec![Arc::new(ImageGenerationTool::new(
            CodexImagesBackend::new(
                create_model_provider(config.provider.clone(), Some(self.auth_manager.clone())),
                thread_store
                    .get::<ThreadOriginator>()
                    .map(|originator| originator.0.clone()),
            ),
            config.save_root.clone(),
            thread_store.level_id().to_string(),
            config.model.clone(),
        ))]
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
