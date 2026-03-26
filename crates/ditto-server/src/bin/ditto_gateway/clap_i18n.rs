#[cfg(feature = "gateway")]
use clap::Command;
#[cfg(feature = "gateway")]
use clap::error::{ContextKind, Error, ErrorKind};

#[cfg(feature = "gateway")]
use ditto_core::resources::MESSAGE_CATALOG;
#[cfg(feature = "gateway")]
use i18n_kit::{Locale, TemplateArg};

#[cfg(feature = "gateway")]
#[derive(Debug)]
pub(crate) struct LocalizedCliError(String);

#[cfg(feature = "gateway")]
impl LocalizedCliError {
    pub(crate) fn new(message: String) -> Self {
        Self(message)
    }
}

#[cfg(feature = "gateway")]
impl std::fmt::Display for LocalizedCliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(feature = "gateway")]
impl std::error::Error for LocalizedCliError {}

#[cfg(feature = "gateway")]
pub(crate) fn localize_clap_command(command: Command, locale: Locale) -> Command {
    let mut command = command;
    command.build();
    localize_clap_tree(command, locale)
}

#[cfg(feature = "gateway")]
fn localize_clap_tree(command: Command, locale: Locale) -> Command {
    let template = localized_help_template(&command, locale);
    let mut command = command.help_template(template);

    if has_arg(&command, "help") {
        command = command.mut_arg("help", |arg| {
            arg.help(clap_help_flag_help(locale))
                .long_help(clap_help_flag_long_help(locale))
        });
    }
    if has_arg(&command, "version") {
        command = command.mut_arg("version", |arg| arg.help(clap_version_flag_help(locale)));
    }
    if command.get_name() == "help" {
        command = command.about(clap_help_command_about(locale));
        if has_arg(&command, "subcommand") {
            command = command.mut_arg("subcommand", |arg| {
                arg.help(clap_help_subcommand_arg_help(locale))
            });
        }
    }
    command.mut_subcommands(|subcommand| localize_clap_tree(subcommand, locale))
}

#[cfg(feature = "gateway")]
pub(crate) fn render_clap_error(command: &mut Command, error: Error, locale: Locale) -> String {
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => error.to_string(),
        _ => render_localized_clap_failure(command, &error, locale),
    }
}

#[cfg(feature = "gateway")]
fn localized_help_template(command: &Command, locale: Locale) -> String {
    let mut template = String::from("{before-help}{name}");
    if command.get_version().is_some() {
        template.push(' ');
        template.push_str("{version}");
    }
    template.push('\n');
    template.push_str("{about-with-newline}");
    template.push_str(&MESSAGE_CATALOG.render(locale, "clap.usage_heading", &[]));
    template.push(' ');
    template.push_str("{usage}\n");

    if command.get_positionals().any(|arg| !arg.is_hide_set()) {
        template.push('\n');
        template.push_str(&MESSAGE_CATALOG.render(locale, "clap.arguments_heading", &[]));
        template.push('\n');
        template.push_str("{positionals}");
    }
    if command.get_opts().any(|arg| !arg.is_hide_set()) {
        template.push('\n');
        template.push_str(&MESSAGE_CATALOG.render(locale, "clap.options_heading", &[]));
        template.push('\n');
        template.push_str("{options}");
    }
    if command
        .get_subcommands()
        .any(|subcommand| !subcommand.is_hide_set())
    {
        template.push('\n');
        template.push_str(&MESSAGE_CATALOG.render(locale, "clap.commands_heading", &[]));
        template.push('\n');
        template.push_str("{subcommands}");
    }

    template.push_str("{after-help}");
    template
}

#[cfg(feature = "gateway")]
fn render_localized_clap_failure(command: &mut Command, error: &Error, locale: Locale) -> String {
    let mut rendered = localized_error_detail(error, locale);

    if let Some(tip) = localized_tip(error, locale) {
        rendered.push('\n');
        rendered.push_str(&tip);
    }

    if let Some(usage) = localized_usage(command, error, locale) {
        rendered.push_str("\n\n");
        rendered.push_str(&usage);
    }

    rendered
}

#[cfg(feature = "gateway")]
fn localized_error_detail(error: &Error, locale: Locale) -> String {
    match error.kind() {
        ErrorKind::UnknownArgument => MESSAGE_CATALOG.render(
            locale,
            "clap.error.unknown_argument",
            &[TemplateArg::new(
                "arg",
                context_string(error, ContextKind::InvalidArg)
                    .unwrap_or_else(|| clap_placeholder_unknown(locale)),
            )],
        ),
        ErrorKind::InvalidSubcommand => MESSAGE_CATALOG.render(
            locale,
            "clap.error.invalid_subcommand",
            &[TemplateArg::new(
                "subcommand",
                context_string(error, ContextKind::InvalidSubcommand)
                    .unwrap_or_else(|| clap_placeholder_unknown(locale)),
            )],
        ),
        ErrorKind::InvalidValue => MESSAGE_CATALOG.render(
            locale,
            "clap.error.invalid_value",
            &[
                TemplateArg::new(
                    "value",
                    context_string(error, ContextKind::InvalidValue)
                        .unwrap_or_else(|| clap_placeholder_unknown(locale)),
                ),
                TemplateArg::new(
                    "arg",
                    context_string(error, ContextKind::InvalidArg)
                        .unwrap_or_else(|| clap_placeholder_unknown(locale)),
                ),
            ],
        ),
        ErrorKind::MissingRequiredArgument => MESSAGE_CATALOG.render(
            locale,
            "clap.error.missing_required_argument",
            &[TemplateArg::new(
                "args",
                context_string(error, ContextKind::InvalidArg)
                    .unwrap_or_else(|| clap_placeholder_unknown(locale)),
            )],
        ),
        ErrorKind::MissingSubcommand => {
            MESSAGE_CATALOG.render(locale, "clap.error.missing_subcommand", &[])
        }
        ErrorKind::ArgumentConflict => MESSAGE_CATALOG.render(
            locale,
            "clap.error.argument_conflict",
            &[
                TemplateArg::new(
                    "arg",
                    context_string(error, ContextKind::InvalidArg)
                        .unwrap_or_else(|| clap_placeholder_unknown(locale)),
                ),
                TemplateArg::new(
                    "prior_arg",
                    context_string(error, ContextKind::PriorArg)
                        .unwrap_or_else(|| clap_placeholder_unknown(locale)),
                ),
            ],
        ),
        ErrorKind::TooFewValues => MESSAGE_CATALOG.render(
            locale,
            "clap.error.too_few_values",
            &[
                TemplateArg::new(
                    "arg",
                    context_string(error, ContextKind::InvalidArg)
                        .unwrap_or_else(|| clap_placeholder_unknown(locale)),
                ),
                TemplateArg::new(
                    "min_values",
                    context_string(error, ContextKind::MinValues)
                        .unwrap_or_else(|| clap_placeholder_unspecified(locale)),
                ),
                TemplateArg::new(
                    "actual_num_values",
                    context_string(error, ContextKind::ActualNumValues)
                        .unwrap_or_else(|| clap_placeholder_unspecified(locale)),
                ),
            ],
        ),
        ErrorKind::TooManyValues => MESSAGE_CATALOG.render(
            locale,
            "clap.error.too_many_values",
            &[TemplateArg::new(
                "arg",
                context_string(error, ContextKind::InvalidArg)
                    .unwrap_or_else(|| clap_placeholder_unknown(locale)),
            )],
        ),
        ErrorKind::WrongNumberOfValues => MESSAGE_CATALOG.render(
            locale,
            "clap.error.wrong_number_of_values",
            &[
                TemplateArg::new(
                    "arg",
                    context_string(error, ContextKind::InvalidArg)
                        .unwrap_or_else(|| clap_placeholder_unknown(locale)),
                ),
                TemplateArg::new(
                    "expected_num_values",
                    context_string(error, ContextKind::ExpectedNumValues)
                        .unwrap_or_else(|| clap_placeholder_unspecified(locale)),
                ),
                TemplateArg::new(
                    "actual_num_values",
                    context_string(error, ContextKind::ActualNumValues)
                        .unwrap_or_else(|| clap_placeholder_unspecified(locale)),
                ),
            ],
        ),
        ErrorKind::NoEquals => MESSAGE_CATALOG.render(
            locale,
            "clap.error.no_equals",
            &[TemplateArg::new(
                "arg",
                context_string(error, ContextKind::InvalidArg)
                    .unwrap_or_else(|| clap_placeholder_unknown(locale)),
            )],
        ),
        ErrorKind::ValueValidation => MESSAGE_CATALOG.render(
            locale,
            "clap.error.value_validation",
            &[
                TemplateArg::new(
                    "arg",
                    context_string(error, ContextKind::InvalidArg)
                        .unwrap_or_else(|| clap_placeholder_unknown(locale)),
                ),
                TemplateArg::new(
                    "value",
                    context_string(error, ContextKind::InvalidValue)
                        .unwrap_or_else(|| error.to_string()),
                ),
            ],
        ),
        _ => MESSAGE_CATALOG.render(
            locale,
            "clap.error.generic",
            &[TemplateArg::new("message", error.to_string())],
        ),
    }
}

#[cfg(feature = "gateway")]
fn localized_tip(error: &Error, locale: Locale) -> Option<String> {
    if let Some(suggestion) = context_string(error, ContextKind::SuggestedValue)
        .or_else(|| context_string(error, ContextKind::SuggestedSubcommand))
        .or_else(|| context_string(error, ContextKind::SuggestedArg))
    {
        return Some(MESSAGE_CATALOG.render(
            locale,
            "clap.tip.did_you_mean",
            &[TemplateArg::new("suggestion", suggestion)],
        ));
    }

    if let Some(values) = context_string(error, ContextKind::ValidValue) {
        return Some(MESSAGE_CATALOG.render(
            locale,
            "clap.tip.possible_values",
            &[TemplateArg::new("values", values)],
        ));
    }

    context_string(error, ContextKind::ValidSubcommand).map(|values| {
        MESSAGE_CATALOG.render(
            locale,
            "clap.tip.available_subcommands",
            &[TemplateArg::new("values", values)],
        )
    })
}

#[cfg(feature = "gateway")]
fn localized_usage(command: &mut Command, error: &Error, locale: Locale) -> Option<String> {
    context_string(error, ContextKind::Usage)
        .or_else(|| {
            let usage = command.render_usage().to_string();
            let usage = usage.trim();
            if usage.is_empty() {
                None
            } else {
                Some(usage.to_string())
            }
        })
        .map(|usage| {
            let heading = MESSAGE_CATALOG.render(locale, "clap.usage_heading", &[]);
            usage.replacen("Usage:", &heading, 1)
        })
}

#[cfg(feature = "gateway")]
fn context_string(error: &Error, kind: ContextKind) -> Option<String> {
    error
        .get(kind)
        .map(ToString::to_string)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(feature = "gateway")]
fn has_arg(command: &Command, id: &str) -> bool {
    command
        .get_arguments()
        .any(|arg| arg.get_id().as_str() == id)
}

#[cfg(feature = "gateway")]
fn clap_help_command_about(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "clap.help_command_about", &[])
}

#[cfg(feature = "gateway")]
fn clap_help_flag_help(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "clap.help_flag.help", &[])
}

#[cfg(feature = "gateway")]
fn clap_help_flag_long_help(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "clap.help_flag.long_help", &[])
}

#[cfg(feature = "gateway")]
fn clap_version_flag_help(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "clap.version_flag.help", &[])
}

#[cfg(feature = "gateway")]
fn clap_help_subcommand_arg_help(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "clap.help_subcommand.arg_help", &[])
}

#[cfg(feature = "gateway")]
fn clap_placeholder_unknown(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "clap.placeholder.unknown", &[])
}

#[cfg(feature = "gateway")]
fn clap_placeholder_unspecified(locale: Locale) -> String {
    MESSAGE_CATALOG.render(locale, "clap.placeholder.unspecified", &[])
}

#[cfg(all(test, feature = "gateway"))]
mod tests {
    use super::*;
    use clap::{Arg, Command};

    #[test]
    fn localizes_help_headings() {
        let mut command = localize_clap_command(
            Command::new("demo")
                .arg(Arg::new("name").required(true))
                .arg(Arg::new("json").long("json"))
                .subcommand(Command::new("show")),
            Locale::ZH_CN,
        );
        let help = command.render_help().to_string();
        assert!(help.contains("用法："));
        assert!(help.contains("参数："));
        assert!(help.contains("选项："));
        assert!(help.contains("命令："));
        assert!(help.contains("打印当前帮助信息"));
        assert!(!help.contains("Print this message"));
    }
}
