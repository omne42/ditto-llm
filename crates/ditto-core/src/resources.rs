use std::borrow::Cow;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use i18n_kit::{Catalog, FallbackStrategy, Locale, TemplateArg, TranslationCatalog};
use i18n_runtime_kit::{
    CatalogInitError, CatalogLocaleError, GlobalCatalog, bootstrap_i18n_catalog,
};
use text_assets_kit::{DataRootOptions, ResourceManifest, TextResource, ensure_data_root};

use crate::error::{DittoError, Result};

pub const DATA_ROOT_DIR_NAME: &str = ".omne_data";
pub const DATA_ROOT_ENV_VAR: &str = "OMNE_DATA_DIR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDefaultFile {
    file_name: &'static str,
    contents: Cow<'static, str>,
}

impl RuntimeDefaultFile {
    #[must_use]
    pub fn new(file_name: &'static str, contents: impl Into<Cow<'static, str>>) -> Self {
        Self {
            file_name,
            contents: contents.into(),
        }
    }

    #[must_use]
    pub const fn file_name(&self) -> &'static str {
        self.file_name
    }

    #[must_use]
    pub fn contents(&self) -> &str {
        self.contents.as_ref()
    }
}

pub struct RuntimeMessageCatalog {
    inner: GlobalCatalog,
    initializer:
        Box<dyn Fn() -> std::result::Result<Arc<dyn Catalog>, CatalogInitError> + Send + Sync>,
    init_state: Mutex<Option<std::result::Result<(), CatalogInitError>>>,
}

pub static MESSAGE_CATALOG: LazyLock<RuntimeMessageCatalog> =
    LazyLock::new(|| RuntimeMessageCatalog::new(load_default_message_catalog));

impl RuntimeMessageCatalog {
    pub fn new<I>(initializer: I) -> Self
    where
        I: Fn() -> std::result::Result<Arc<dyn Catalog>, CatalogInitError> + Send + Sync + 'static,
    {
        Self {
            inner: GlobalCatalog::new(Locale::EN_US),
            initializer: Box::new(initializer),
            init_state: Mutex::new(None),
        }
    }

    pub fn replace<C>(&self, catalog: C)
    where
        C: Catalog + 'static,
    {
        self.inner.replace(catalog);
        *self
            .init_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(Ok(()));
    }

    pub fn with_catalog<T>(
        &self,
        f: impl FnOnce(&dyn Catalog) -> T,
    ) -> std::result::Result<T, CatalogInitError> {
        self.ensure_initialized()?;
        self.inner.with_catalog(f)
    }

    pub fn render(&self, locale: Locale, key: &str, args: &[TemplateArg<'_>]) -> String {
        self.with_catalog(|catalog| catalog.render_text(locale, key, args))
            .unwrap_or_else(|_| key.to_string())
    }

    pub fn resolve_cli_locale(
        &self,
        args: Vec<String>,
        env_var: &str,
    ) -> std::result::Result<(Locale, Vec<String>), CatalogLocaleError> {
        self.ensure_initialized()
            .map_err(CatalogLocaleError::Initialization)?;
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

impl RuntimeMessageCatalog {
    fn ensure_initialized(&self) -> std::result::Result<(), CatalogInitError> {
        let mut init_state = self
            .init_state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(result) = init_state.clone() {
            return result;
        }

        let catalog = (self.initializer)()?;
        self.inner.replace(InstalledCatalog(catalog));
        let result = Ok(());
        *init_state = Some(result.clone());
        result
    }
}

struct InstalledCatalog(Arc<dyn Catalog>);

impl TranslationCatalog for InstalledCatalog {
    fn resolve_shared(&self, locale: Locale, key: &str) -> i18n_kit::TranslationResolution {
        self.0.resolve_shared(locale, key)
    }
}

impl Catalog for InstalledCatalog {
    fn default_locale(&self) -> Locale {
        self.0.default_locale()
    }

    fn available_locales(&self) -> Vec<Locale> {
        self.0.available_locales()
    }

    fn locale_enabled(&self, locale: Locale) -> bool {
        self.0.locale_enabled(locale)
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

pub fn runtime_data_root_options(
    data_dir: Option<PathBuf>,
    scope: text_assets_kit::DataRootScope,
) -> DataRootOptions {
    let options = runtime_assets_data_root_options().with_scope(scope);
    match data_dir {
        Some(data_dir) => options.with_data_dir(data_dir),
        None => options,
    }
}

pub fn sniff_runtime_data_root_options(args: &[String]) -> Result<DataRootOptions> {
    let mut data_dir = None;
    let mut scope = text_assets_kit::DataRootScope::Auto;
    let mut iter = args.iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--root" => {
                let value = iter.next().ok_or_else(|| {
                    crate::config_error!(
                        "error_detail.config.missing_flag_value",
                        "flag" => "--root"
                    )
                })?;
                if value.trim().is_empty() {
                    return Err(crate::config_error!(
                        "error_detail.config.empty_flag_value",
                        "flag" => "--root"
                    ));
                }
                data_dir = Some(PathBuf::from(value));
            }
            "--scope" => {
                let value = iter.next().ok_or_else(|| {
                    crate::config_error!(
                        "error_detail.config.missing_flag_value",
                        "flag" => "--scope"
                    )
                })?;
                scope = parse_data_root_scope(value)?;
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--root=") {
                    if value.trim().is_empty() {
                        return Err(crate::config_error!(
                            "error_detail.config.empty_flag_value",
                            "flag" => "--root"
                        ));
                    }
                    data_dir = Some(PathBuf::from(value));
                    continue;
                }
                if let Some(value) = arg.strip_prefix("--scope=") {
                    scope = parse_data_root_scope(value)?;
                }
            }
        }
    }

    Ok(runtime_data_root_options(data_dir, scope))
}

pub fn bootstrap_cli_runtime_with_options(options: &DataRootOptions) -> Result<RuntimeAssets> {
    bootstrap_runtime_assets_with_options(options)
}

pub fn bootstrap_cli_runtime_from_args(args: &[String]) -> Result<RuntimeAssets> {
    let options = sniff_runtime_data_root_options(args)?;
    bootstrap_cli_runtime_with_options(&options)
}

pub fn bootstrap_cli_runtime_with_options_and_defaults<I>(
    options: &DataRootOptions,
    default_files: I,
) -> Result<RuntimeAssets>
where
    I: IntoIterator<Item = RuntimeDefaultFile>,
{
    let runtime_assets = bootstrap_cli_runtime_with_options(options)?;
    bootstrap_runtime_default_files(runtime_assets.data_root(), default_files)?;
    Ok(runtime_assets)
}

pub fn bootstrap_cli_runtime_from_args_with_defaults<I>(
    args: &[String],
    default_files: I,
) -> Result<RuntimeAssets>
where
    I: IntoIterator<Item = RuntimeDefaultFile>,
{
    let options = sniff_runtime_data_root_options(args)?;
    bootstrap_cli_runtime_with_options_and_defaults(&options, default_files)
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

pub fn bootstrap_runtime_default_files<I>(data_root: &Path, default_files: I) -> Result<()>
where
    I: IntoIterator<Item = RuntimeDefaultFile>,
{
    for file in default_files {
        write_if_missing(data_root.join(file.file_name()), file.contents())
            .map_err(DittoError::Io)?;
    }
    Ok(())
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

fn parse_data_root_scope(value: &str) -> Result<text_assets_kit::DataRootScope> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(text_assets_kit::DataRootScope::Auto),
        "workspace" => Ok(text_assets_kit::DataRootScope::Workspace),
        "global" => Ok(text_assets_kit::DataRootScope::Global),
        _ => Err(crate::config_error!(
            "error_detail.config.invalid_flag_value",
            "flag" => "--scope",
            "value" => value
        )),
    }
}

fn write_if_missing(path: PathBuf, contents: &str) -> std::io::Result<()> {
    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
    {
        Ok(mut file) => file.write_all(contents.as_bytes()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use i18n_kit::{Locale, StaticJsonCatalog, StaticJsonLocale, TemplateArg};
    use i18n_runtime_kit::CatalogInitError;
    use std::sync::Arc;

    #[test]
    fn runtime_message_catalog_initializes_on_first_render() {
        static SOURCES: [StaticJsonLocale; 1] = [StaticJsonLocale::new(
            Locale::EN_US,
            true,
            r#"{"hello":"hello {name}"}"#,
        )];
        let catalog = RuntimeMessageCatalog::new(|| {
            let catalog = StaticJsonCatalog::try_new(Locale::EN_US, &SOURCES)
                .expect("static catalog should be valid");
            Ok(Arc::new(catalog))
        });

        let rendered = catalog.render(Locale::EN_US, "hello", &[TemplateArg::new("name", "Alice")]);
        assert_eq!(rendered, "hello Alice");
    }

    #[test]
    fn runtime_message_catalog_replace_overrides_failed_initializer() {
        static SOURCES: [StaticJsonLocale; 1] = [StaticJsonLocale::new(
            Locale::EN_US,
            true,
            r#"{"hello":"hello"}"#,
        )];
        let catalog = RuntimeMessageCatalog::new(|| {
            Err(CatalogInitError::new(std::io::Error::other("init failed")))
        });
        let replacement =
            StaticJsonCatalog::try_new(Locale::EN_US, &SOURCES).expect("static catalog valid");

        catalog.replace(replacement);

        assert_eq!(catalog.render(Locale::EN_US, "hello", &[]), "hello");
    }

    #[test]
    fn sniff_runtime_data_root_options_reads_root_and_scope_flags() {
        let options = sniff_runtime_data_root_options(&[
            "--root".to_string(),
            "/tmp/omne".to_string(),
            "--scope".to_string(),
            "workspace".to_string(),
        ])
        .expect("parse data root options");

        let debug = format!("{options:?}");
        assert!(debug.contains("data_dir: Some(\"/tmp/omne\")"));
        assert!(debug.contains("scope: Workspace"));
    }

    #[test]
    fn sniff_runtime_data_root_options_rejects_missing_root_value() {
        let err =
            sniff_runtime_data_root_options(&["--root".to_string()]).expect_err("missing value");
        let DittoError::Config(message) = err else {
            panic!("expected config error");
        };
        assert_eq!(
            message.as_catalog().map(|message| message.code()),
            Some("error_detail.config.missing_flag_value")
        );
    }

    #[test]
    fn sniff_runtime_data_root_options_rejects_invalid_scope() {
        let err = sniff_runtime_data_root_options(&["--scope=sideways".to_string()])
            .expect_err("invalid scope");
        let DittoError::Config(message) = err else {
            panic!("expected config error");
        };
        assert_eq!(
            message.as_catalog().map(|message| message.code()),
            Some("error_detail.config.invalid_flag_value")
        );
    }

    #[test]
    fn bootstrap_runtime_default_files_only_creates_missing_files() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        let existing_path = root.join("config.toml");
        std::fs::write(&existing_path, "user = true\n").expect("seed existing config");

        bootstrap_runtime_default_files(
            root,
            [
                RuntimeDefaultFile::new("config.toml", "default = true\n"),
                RuntimeDefaultFile::new("gateway.json", "{}\n"),
            ],
        )
        .expect("bootstrap default files");

        assert_eq!(
            std::fs::read_to_string(&existing_path).expect("read existing config"),
            "user = true\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("gateway.json")).expect("read gateway config"),
            "{}\n"
        );
    }
}
