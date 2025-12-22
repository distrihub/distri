//!
//! Distri TypeScript Import Provider
//!
//! Handles custom import resolution for Distri TypeScript plugins:
//! - Resolves relative imports from plugin's src/ directory
//! - Maps distri/base.ts and other runtime modules
//! - Handles local file imports within plugin structure
//!

use anyhow::Result;
use deno_error::JsErrorBox;
use rustyscript::{
    deno_core::{error::ModuleLoaderError, ModuleSpecifier, RequestedModuleType, ResolutionKind},
    module_loader::ImportProvider,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::warn;

use crate::{executors::ts_executor::types::DISTRI_BASE, plugin_trait::PluginFileResolver};

/// Distri import provider for TypeScript plugins
pub struct DistriImportProvider {
    /// Static modules (e.g., runtime base modules)
    static_modules: HashMap<String, String>,
    /// Cached dynamic modules loaded from plugins
    dynamic_modules: HashMap<String, String>,
    /// Resolver registry for plugins keyed by package name
    resolvers: Arc<RwLock<HashMap<String, Arc<dyn PluginFileResolver>>>>,
}

impl DistriImportProvider {
    fn convert_distri_runtime(specifier: &ModuleSpecifier) -> Option<ModuleSpecifier> {
        const PACKAGE_PREFIX: &str = "@distri/runtime";

        let to_url = |suffix: &str| {
            let trimmed = suffix.trim_start_matches('/');
            if trimmed.is_empty() {
                return None;
            }
            let url = format!("https://jsr.io/{}", trimmed);
            ModuleSpecifier::parse(&url).ok()
        };

        let raw = specifier.as_str();

        if let Some(rest) = raw.strip_prefix("jsr:") {
            let normalized = rest.trim_start_matches('/');
            if normalized.starts_with(PACKAGE_PREFIX) {
                return to_url(rest);
            }
        }

        let normalized = raw.trim_start_matches('/');
        if normalized.starts_with(PACKAGE_PREFIX) {
            return to_url(raw);
        }

        None
    }

    pub fn new(resolvers: Arc<RwLock<HashMap<String, Arc<dyn PluginFileResolver>>>>) -> Self {
        Self {
            static_modules: HashMap::from([(
                format!("/{}", DISTRI_BASE.0),
                DISTRI_BASE.1.to_string(),
            )]),
            dynamic_modules: HashMap::new(),
            resolvers,
        }
    }

    fn plugin_reference_from_file_path(path: &str) -> Option<(String, String)> {
        let marker = "plugin:/";
        let idx = path.find(marker)?;
        let after = &path[idx + marker.len()..];
        let mut segments = after.splitn(2, '/');
        let package = segments.next()?.to_string();
        let module_path = segments.next().unwrap_or("").to_string();
        Some((package, module_path))
    }

    fn plugin_cache_key(package: &str, module_path: &str) -> String {
        if module_path.is_empty() {
            format!("plugin://{}", package)
        } else {
            format!("plugin://{}/{}", package, module_path)
        }
    }

    fn plugin_reference_from_specifier(
        specifier: &ModuleSpecifier,
    ) -> Option<(String, String, String)> {
        match specifier.scheme() {
            "plugin" => {
                let package = specifier.host_str()?.to_string();
                let module_path = specifier.path().trim_start_matches('/').to_string();
                let cache_key = Self::plugin_cache_key(&package, &module_path);
                Some((package, module_path, cache_key))
            }
            "file" => {
                let path = specifier.path();
                let (package, module_path) = Self::plugin_reference_from_file_path(path)?;
                let cache_key = Self::plugin_cache_key(&package, &module_path);
                Some((package, module_path, cache_key))
            }
            _ => None,
        }
    }

    fn normalize_specifier(specifier: &ModuleSpecifier) -> ModuleSpecifier {
        if let Some(converted) = Self::convert_distri_runtime(specifier) {
            return converted;
        }

        if specifier.scheme() == "file" {
            if let Some((package, module_path, cache_key)) =
                Self::plugin_reference_from_specifier(specifier)
            {
                if let Ok(url) = ModuleSpecifier::parse(&cache_key) {
                    return url;
                }
                if let Ok(url) =
                    ModuleSpecifier::parse(&Self::plugin_cache_key(&package, &module_path))
                {
                    return url;
                }
            }
        }
        specifier.clone()
    }

    fn fetch_plugin_module(
        &mut self,
        specifier: &ModuleSpecifier,
    ) -> Option<Result<String, ModuleLoaderError>> {
        let (package, module_path, cache_key) = Self::plugin_reference_from_specifier(specifier)?;

        if let Some(cached) = self.dynamic_modules.get(&cache_key) {
            return Some(Ok(cached.clone()));
        }

        let resolver = {
            let guard = self.resolvers.read().ok()?;
            guard.get(&package).cloned()
        }?;

        let bytes = match resolver.read(&module_path) {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!(
                    "Failed to read plugin module {}:{} -> {}",
                    package, module_path, err
                );
                return Some(Err(JsErrorBox::generic(err.to_string())));
            }
        };

        let source = match String::from_utf8(bytes) {
            Ok(source) => source,
            Err(err) => {
                warn!(
                    "Plugin module {}:{} is not valid UTF-8: {}",
                    package, module_path, err
                );
                return Some(Err(JsErrorBox::generic(err.to_string())));
            }
        };

        self.dynamic_modules.insert(cache_key, source.clone());

        Some(Ok(source))
    }

    fn resolve_plugin_path(
        &self,
        specifier: &ModuleSpecifier,
        referrer: &ModuleSpecifier,
    ) -> Option<ModuleSpecifier> {
        let normalized_referrer = Self::normalize_specifier(referrer);
        if normalized_referrer.scheme() != "plugin" {
            return None;
        }

        // Absolute plugin URLs are returned as-is
        if specifier.scheme() == "plugin" {
            return Some(specifier.clone());
        }

        let normalized_specifier = Self::normalize_specifier(specifier);
        if normalized_specifier.scheme() == "plugin" {
            return Some(normalized_specifier);
        }

        // Use standard URL join for relative paths
        normalized_referrer.join(specifier.as_str()).ok()
    }
}

impl ImportProvider for DistriImportProvider {
    fn resolve(
        &mut self,
        specifier: &ModuleSpecifier,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> Option<Result<ModuleSpecifier, ModuleLoaderError>> {
        let normalized_specifier = Self::normalize_specifier(specifier);

        match normalized_specifier.scheme() {
            "plugin" | "http" | "https" => return Some(Ok(normalized_specifier)),
            _ => {}
        }

        if referrer.is_empty() {
            return None;
        }

        let referrer_spec = match ModuleSpecifier::parse(referrer) {
            Ok(spec) => spec,
            Err(_) => return None,
        };

        let normalized_referrer = Self::normalize_specifier(&referrer_spec);

        match normalized_referrer.scheme() {
            "plugin" => self
                .resolve_plugin_path(specifier, &normalized_referrer)
                .map(Ok),
            "http" | "https" => normalized_referrer.join(specifier.as_str()).ok().map(Ok),
            _ => None,
        }
    }

    fn import(
        &mut self,
        specifier: &ModuleSpecifier,
        _referrer: Option<&ModuleSpecifier>,
        _is_dyn_import: bool,
        _requested_module_type: RequestedModuleType,
    ) -> Option<Result<String, ModuleLoaderError>> {
        if let Some(result) = self.fetch_plugin_module(specifier) {
            return Some(result);
        }

        // Handle distri runtime static modules via domain path
        if let Some(source) = self.static_modules.get(specifier.path()) {
            return Some(Ok(source.clone()));
        }

        None
    }
}
