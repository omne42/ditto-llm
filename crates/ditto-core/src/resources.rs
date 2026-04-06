use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use i18n_kit::{Catalog, FallbackStrategy, Locale, TemplateArg};
#[allow(deprecated)]
use i18n_runtime_kit::{CatalogInitError, CatalogLocaleError, LazyCatalog, bootstrap_i18n_catalog};
use text_assets_kit::{DataRootOptions, ResourceManifest, TextResource, ensure_data_root};

use crate::error::{DittoError, Result};

const DATA_ROOT_DIR_NAME: &str = ".omne_data";
const DATA_ROOT_ENV_VAR: &str = "OMNE_DATA_DIR";

#[allow(deprecated)]
pub struct RuntimeMessageCatalog {
    inner: LazyCatalog,
}

pub static MESSAGE_CATALOG: LazyLock<RuntimeMessageCatalog> =
    LazyLock::new(|| RuntimeMessageCatalog::new(load_default_message_catalog));

#[allow(deprecated)]
impl RuntimeMessageCatalog {
    pub fn new<I>(initializer: I) -> Self
    where
        I: Fn() -> std::result::Result<Arc<dyn Catalog>, CatalogInitError> + Send + Sync + 'static,
    {
        Self {
            inner: LazyCatalog::new(initializer),
        }
    }

    pub fn replace<C>(&self, catalog: C)
    where
        C: Catalog + 'static,
    {
        self.inner.replace(catalog);
    }

    pub fn with_catalog<T>(
        &self,
        f: impl FnOnce(&dyn Catalog) -> T,
    ) -> std::result::Result<T, CatalogInitError> {
        self.inner.with_catalog(f)
    }

    pub fn render(&self, locale: Locale, key: &str, args: &[TemplateArg<'_>]) -> String {
        self.inner
            .try_render(locale, key, args)
            .unwrap_or_else(|_| key.to_string())
    }

    pub fn resolve_cli_locale(
        &self,
        args: Vec<String>,
        env_var: &str,
    ) -> std::result::Result<(Locale, Vec<String>), CatalogLocaleError> {
        self.inner.resolve_locale_from_cli_args(args, env_var)
    }

    pub fn default_locale(&self) -> Option<Locale> {
        self.with_catalog(|catalog| catalog.default_locale()).ok()
    }

    pub fn locale_enabled(&self, locale: Locale) -> Option<bool> {
        self.with_catalog(|catalog| catalog.locale_enabled(locale))
            .ok()
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeAssets {
    data_root: PathBuf,
    i18n_dir: PathBuf,
}

impl RuntimeAssets {
    #[must_use]
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    #[must_use]
    pub fn i18n_dir(&self) -> &Path {
        &self.i18n_dir
    }
}

pub fn bootstrap_runtime_assets() -> Result<RuntimeAssets> {
    bootstrap_runtime_assets_with_options(&runtime_assets_data_root_options())
}

pub fn bootstrap_runtime_assets_at_root(root: impl Into<PathBuf>) -> Result<RuntimeAssets> {
    let options = runtime_assets_data_root_options().with_data_dir(root.into());
    bootstrap_runtime_assets_with_options(&options)
}

pub fn bootstrap_runtime_assets_with_options(options: &DataRootOptions) -> Result<RuntimeAssets> {
    let data_root = ensure_data_root(options).map_err(DittoError::Io)?;
    let i18n_dir = data_root.join("i18n");

    let i18n_catalog = bootstrap_i18n_catalog(
        &i18n_dir,
        &default_i18n_manifest(),
        Locale::EN_US,
        FallbackStrategy::Both,
    )
    .map_err(|err| {
        crate::config_error!(
            "error_detail.config.i18n_catalog_bootstrap_failed",
            "error" => err.to_string()
        )
    })?;
    MESSAGE_CATALOG.replace(i18n_catalog);

    Ok(RuntimeAssets {
        data_root,
        i18n_dir,
    })
}

fn load_default_message_catalog() -> std::result::Result<Arc<dyn Catalog>, CatalogInitError> {
    let data_root =
        ensure_data_root(&runtime_assets_data_root_options()).map_err(CatalogInitError::from)?;
    let i18n_dir = data_root.join("i18n");
    let catalog = bootstrap_i18n_catalog(
        &i18n_dir,
        &default_i18n_manifest(),
        Locale::EN_US,
        FallbackStrategy::Both,
    )
    .map_err(CatalogInitError::new)?;
    Ok(Arc::new(catalog))
}

fn runtime_assets_data_root_options() -> DataRootOptions {
    DataRootOptions::default()
        .with_dir_name(DATA_ROOT_DIR_NAME)
        .with_env_var(DATA_ROOT_ENV_VAR)
}

fn default_i18n_manifest() -> ResourceManifest {
    ResourceManifest::new()
        .with_resource(
            TextResource::new("en_US.json", include_str!("../i18n/en_US.json"))
                .expect("embedded en_US catalog should be valid"),
        )
        .with_resource(
            TextResource::new("zh_CN.json", include_str!("../i18n/zh_CN.json"))
                .expect("embedded zh_CN catalog should be valid"),
        )
        .with_resource(
            TextResource::new("ja_JP.json", include_str!("../i18n/ja_JP.json"))
                .expect("embedded ja_JP catalog should be valid"),
        )
}
