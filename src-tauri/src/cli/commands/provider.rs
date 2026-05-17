use clap::Subcommand;
use std::path::PathBuf;

use super::provider_inspect;
use crate::app_config::AppType;
use crate::cli::commands::provider_input::{
    common_snippet_has_effective_config, current_timestamp, display_provider_summary,
    generate_provider_id, prompt_basic_fields, prompt_optional_fields, prompt_settings_config,
    prompt_settings_config_for_add, provider_uses_common_config, set_provider_common_config_meta,
    supports_common_config, OptionalFields, ProviderAddMode,
};
use crate::cli::i18n::texts;
use crate::cli::ui::{error, highlight, info, success, warning};
use crate::error::AppError;
use crate::provider::{AuthBinding, AuthBindingSource, Provider, ProviderMeta};
use crate::services::ProviderService;
use crate::store::AppState;
use inquire::{Confirm, Select, Text};
use serde_json::json;

const CODEX_OAUTH_PROVIDER_TYPE: &str = "codex_oauth";
const CODEX_OAUTH_PROVIDER_TYPE_CLI: &str = "codex-oauth";
const CODEX_OAUTH_API_FORMAT: &str = "openai_responses";
const CODEX_OAUTH_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const CODEX_OAUTH_WEBSITE_URL: &str = "https://openai.com/chatgpt/pricing";
const CODEX_OAUTH_DEFAULT_MODEL: &str = "gpt-5.4";
const CODEX_OAUTH_DEFAULT_HAIKU_MODEL: &str = "gpt-5.4-mini";

fn supports_official_provider(app_type: &AppType) -> bool {
    matches!(app_type, AppType::Codex)
}

fn is_codex_official_provider(provider: &Provider) -> bool {
    provider
        .meta
        .as_ref()
        .and_then(|meta| meta.codex_official)
        .unwrap_or(false)
        || provider.category.as_deref() == Some("official")
        || provider.website_url.as_deref() == Some("https://chatgpt.com/codex")
        || provider.name.trim().eq_ignore_ascii_case("OpenAI Official")
}

fn prompt_common_config_enabled(
    app_type: &AppType,
    common_snippet: Option<&str>,
    current: Option<&Provider>,
) -> Result<Option<bool>, AppError> {
    if !supports_common_config(app_type)
        || !common_snippet_has_effective_config(app_type, common_snippet)
    {
        return Ok(None);
    }

    let default_enabled = current
        .map(|provider| provider_uses_common_config(app_type, provider, common_snippet))
        .unwrap_or(true);
    let enabled = Confirm::new(texts::tui_form_attach_common_config())
        .with_default(default_enabled)
        .prompt()
        .map_err(|e| AppError::Message(texts::input_failed_error(&e.to_string())))?;
    Ok(Some(enabled))
}

#[derive(Subcommand)]
pub enum ProviderCommand {
    /// List all providers
    List,
    /// Show current provider
    Current,
    /// Switch to a provider
    Switch {
        /// Provider ID to switch to
        id: String,
    },
    /// Add a new provider
    Add {
        /// Provider type for non-interactive creation, for example codex-oauth
        #[arg(long = "type", value_parser = ["codex-oauth"])]
        provider_type: Option<String>,
        /// Provider display name for non-interactive creation
        #[arg(long)]
        name: Option<String>,
        /// Managed auth account ID, or default to follow the default account
        #[arg(long)]
        account: Option<String>,
    },
    /// Edit a provider
    Edit {
        /// Provider ID to edit
        id: String,
    },
    /// Delete a provider
    Delete {
        /// Provider ID to delete
        id: String,
    },
    /// Duplicate a provider
    Duplicate {
        /// Provider ID to duplicate
        id: String,
    },
    /// Test provider endpoint speed
    Speedtest {
        /// Provider ID to test
        id: String,
    },
    /// Run stream health check for a provider
    StreamCheck {
        /// Provider ID to check
        id: String,
    },
    /// Fetch remote model list for a provider
    FetchModels {
        /// Provider ID to query
        id: String,
    },
    /// Export a Claude provider to a standalone settings file
    Export {
        /// Provider ID to export
        id: String,
        /// Output path (default: {cwd}/.claude/settings.local.json)
        /// If path is a directory, appends settings-{provider-name}.json
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

pub fn execute(cmd: ProviderCommand, app: Option<AppType>) -> Result<(), AppError> {
    let app_type = app.unwrap_or(AppType::Claude);

    match cmd {
        ProviderCommand::List => provider_inspect::list_providers(app_type),
        ProviderCommand::Current => provider_inspect::show_current(app_type),
        ProviderCommand::Switch { id } => switch_provider(app_type, &id),
        ProviderCommand::Add {
            provider_type,
            name,
            account,
        } => add_provider(app_type, provider_type, name, account),
        ProviderCommand::Edit { id } => edit_provider(app_type, &id),
        ProviderCommand::Delete { id } => delete_provider(app_type, &id),
        ProviderCommand::Duplicate { id } => duplicate_provider(app_type, &id),
        ProviderCommand::Speedtest { id } => provider_inspect::speedtest_provider(app_type, &id),
        ProviderCommand::StreamCheck { id } => {
            provider_inspect::stream_check_provider(app_type, &id)
        }
        ProviderCommand::FetchModels { id } => {
            provider_inspect::fetch_models_provider(app_type, &id)
        }
        ProviderCommand::Export { id, output } => export_provider(app_type, &id, output),
    }
}

fn get_state() -> Result<AppState, AppError> {
    AppState::try_new()
}

fn switch_provider(app_type: AppType, id: &str) -> Result<(), AppError> {
    let state = get_state()?;
    let app_str = app_type.as_str().to_string();
    let skip_live_sync = !crate::sync_policy::should_sync_live(&app_type);

    // 检查 provider 是否存在
    let providers = ProviderService::list(&state, app_type.clone())?;
    let Some(provider) = providers.get(id).cloned() else {
        return Err(AppError::Message(format!("Provider '{}' not found", id)));
    };

    // 执行切换
    ProviderService::switch(&state, app_type.clone(), id)?;
    if let Err(err) =
        crate::claude_plugin::sync_claude_plugin_on_provider_switch(&app_type, &provider)
    {
        println!(
            "{}",
            warning(&texts::claude_plugin_sync_failed_warning(&err.to_string()))
        );
    }

    if app_type.is_additive_mode() {
        println!(
            "{}",
            success(&texts::provider_added_to_app_config(id, &app_str))
        );
    } else {
        println!("{}", success(&texts::switched_to_provider(id)));
    }
    println!("{}", info(&format!("  Application: {}", app_str)));
    if skip_live_sync {
        println!(
            "{}",
            warning(&texts::live_sync_skipped_uninitialized_warning(&app_str))
        );
    }
    println!("\n{}", info(texts::restart_note()));

    Ok(())
}

fn delete_provider(app_type: AppType, id: &str) -> Result<(), AppError> {
    let state = get_state()?;

    // 检查是否是当前 provider
    let current_id = ProviderService::current(&state, app_type.clone())?;
    if id == current_id {
        return Err(AppError::Message(
            "Cannot delete the current active provider. Please switch to another provider first."
                .to_string(),
        ));
    }

    // 确认删除
    let confirm = inquire::Confirm::new(&format!(
        "Are you sure you want to delete provider '{}'?",
        id
    ))
    .with_default(false)
    .prompt()
    .map_err(|e| AppError::Message(format!("Prompt failed: {}", e)))?;

    if !confirm {
        println!("{}", info("Cancelled."));
        return Ok(());
    }

    // 执行删除
    ProviderService::delete(&state, app_type, id)?;

    println!("{}", success(&format!("✓ Deleted provider '{}'", id)));

    Ok(())
}

fn add_provider(
    app_type: AppType,
    provider_type: Option<String>,
    name: Option<String>,
    account: Option<String>,
) -> Result<(), AppError> {
    if provider_type.is_some() || name.is_some() || account.is_some() {
        return add_provider_non_interactive(app_type, provider_type, name, account);
    }

    // Disable bracketed paste mode to work around inquire dropping paste events
    crate::cli::terminal::disable_bracketed_paste_mode_best_effort();

    println!("{}", highlight("Add New Provider"));
    println!("{}", "=".repeat(50));

    let add_mode = if supports_official_provider(&app_type) {
        let choices = vec![
            texts::add_official_provider(),
            texts::add_third_party_provider(),
        ];
        match Select::new(texts::select_provider_add_mode(), choices.clone()).prompt() {
            Ok(selected) if selected == texts::add_official_provider() => ProviderAddMode::Official,
            Ok(_selected) => ProviderAddMode::ThirdParty,
            Err(inquire::error::InquireError::OperationCanceled)
            | Err(inquire::error::InquireError::OperationInterrupted) => {
                println!("{}", info(texts::cancelled()));
                return Ok(());
            }
            Err(e) => {
                return Err(AppError::Message(texts::input_failed_error(&e.to_string())));
            }
        }
    } else {
        ProviderAddMode::ThirdParty
    };

    // 1. 加载配置和状态
    let state = AppState::try_new()?;
    let config = state.config.read().unwrap();
    let manager = config
        .get_manager(&app_type)
        .ok_or_else(|| AppError::Message(texts::app_config_not_found(app_type.as_str())))?;
    let existing_ids: Vec<String> = manager.providers.keys().cloned().collect();
    let common_snippet = config.common_config_snippets.get(&app_type).cloned();
    drop(config);

    // 2. 收集基本字段
    let is_codex_official = matches!(
        (app_type.clone(), add_mode),
        (AppType::Codex, ProviderAddMode::Official)
    );
    let (name, website_url) = match (app_type.clone(), add_mode) {
        (AppType::Codex, ProviderAddMode::Official) => {
            let name = Text::new(texts::provider_name_label())
                .with_placeholder("OpenAI")
                .with_help_message(texts::provider_name_help())
                .prompt()
                .map_err(|e| AppError::Message(texts::input_failed_error(&e.to_string())))?;
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err(AppError::InvalidInput(
                    texts::provider_name_empty_error().to_string(),
                ));
            }
            (name, Some("https://chatgpt.com/codex".to_string()))
        }
        _ => prompt_basic_fields(None)?,
    };
    let id = generate_provider_id(&name, &existing_ids);
    println!("{}", info(&texts::generated_id_message(&id)));

    // 3. 收集配置
    let settings_config = prompt_settings_config_for_add(&app_type, add_mode)?;

    // 4. 询问是否配置可选字段
    let optional = if Confirm::new(texts::configure_optional_fields_prompt())
        .with_default(false)
        .prompt()
        .map_err(|e| AppError::Message(texts::input_failed_error(&e.to_string())))?
    {
        prompt_optional_fields(None)?
    } else {
        OptionalFields::default()
    };

    // 5. 构建 Provider 对象
    let mut provider = Provider {
        id: id.clone(),
        name,
        settings_config,
        website_url,
        category: None,
        created_at: Some(current_timestamp()),
        sort_index: optional.sort_index,
        notes: optional.notes,
        icon: None,
        icon_color: None,
        meta: if is_codex_official {
            Some(ProviderMeta {
                codex_official: Some(true),
                ..Default::default()
            })
        } else {
            None
        },
        in_failover_queue: false,
    };
    if let Some(enabled) = prompt_common_config_enabled(&app_type, common_snippet.as_deref(), None)?
    {
        set_provider_common_config_meta(&mut provider, enabled);
    }

    // 6. 显示摘要并确认
    display_provider_summary(&provider, &app_type);
    if !Confirm::new(&texts::confirm_create_entity(texts::entity_provider()))
        .with_default(false)
        .prompt()
        .map_err(|e| AppError::Message(texts::input_failed_error(&e.to_string())))?
    {
        println!("{}", info(texts::cancelled()));
        return Ok(());
    }

    // 7. 调用 Service 层
    ProviderService::add(&state, app_type.clone(), provider)?;

    // 8. 成功消息
    println!(
        "\n{}",
        success(&texts::entity_added_success(texts::entity_provider(), &id))
    );

    Ok(())
}

fn add_provider_non_interactive(
    app_type: AppType,
    provider_type: Option<String>,
    name: Option<String>,
    account: Option<String>,
) -> Result<(), AppError> {
    if !matches!(app_type, AppType::Claude) {
        return Err(AppError::Message(format!(
            "Non-interactive codex-oauth provider creation currently supports only Claude. Use --app claude (current app: {}).",
            app_type.as_str()
        )));
    }

    let provider_type = provider_type.ok_or_else(|| {
        AppError::Message(format!(
            "Missing --type. Supported value: {CODEX_OAUTH_PROVIDER_TYPE_CLI}"
        ))
    })?;
    if !matches!(
        provider_type.trim().to_ascii_lowercase().as_str(),
        "codex-oauth" | "codex_oauth" | "codexoauth"
    ) {
        return Err(AppError::Message(format!(
            "Unsupported provider type: {provider_type}. Supported value: {CODEX_OAUTH_PROVIDER_TYPE_CLI}"
        )));
    }

    let name = name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Message("Missing --name for codex-oauth provider".to_string()))?;
    let account_id = account
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "default");

    let state = get_state()?;
    let existing_ids: Vec<String> = {
        let config = state.config.read().map_err(AppError::from)?;
        let manager = config
            .get_manager(&app_type)
            .ok_or_else(|| AppError::Message(texts::app_config_not_found(app_type.as_str())))?;
        manager.providers.keys().cloned().collect()
    };
    let id = generate_provider_id(&name, &existing_ids);
    let provider = build_codex_oauth_provider(&id, &name, account_id);

    ProviderService::add(&state, app_type, provider)?;
    println!(
        "{}",
        success(&texts::entity_added_success(texts::entity_provider(), &id))
    );
    println!(
        "{}",
        info(&format!("Auth provider: {CODEX_OAUTH_PROVIDER_TYPE_CLI}"))
    );
    if let Some(account_id) = account_id {
        println!("{}", info(&format!("Account: {account_id}")));
    } else {
        println!("{}", info("Account: default"));
    }

    Ok(())
}

fn build_codex_oauth_provider(id: &str, name: &str, account_id: Option<&str>) -> Provider {
    Provider {
        id: id.to_string(),
        name: name.to_string(),
        settings_config: json!({
            "env": {
                "ANTHROPIC_BASE_URL": CODEX_OAUTH_BASE_URL,
                "ANTHROPIC_MODEL": CODEX_OAUTH_DEFAULT_MODEL,
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": CODEX_OAUTH_DEFAULT_HAIKU_MODEL,
                "ANTHROPIC_DEFAULT_SONNET_MODEL": CODEX_OAUTH_DEFAULT_MODEL,
                "ANTHROPIC_DEFAULT_OPUS_MODEL": CODEX_OAUTH_DEFAULT_MODEL
            }
        }),
        website_url: Some(CODEX_OAUTH_WEBSITE_URL.to_string()),
        category: Some("third_party".to_string()),
        created_at: Some(current_timestamp()),
        sort_index: None,
        notes: None,
        icon: None,
        icon_color: None,
        meta: Some(ProviderMeta {
            provider_type: Some(CODEX_OAUTH_PROVIDER_TYPE.to_string()),
            api_format: Some(CODEX_OAUTH_API_FORMAT.to_string()),
            auth_binding: Some(AuthBinding {
                source: AuthBindingSource::ManagedAccount,
                auth_provider: Some(CODEX_OAUTH_PROVIDER_TYPE.to_string()),
                account_id: account_id.map(str::to_string),
            }),
            ..Default::default()
        }),
        in_failover_queue: false,
    }
}

fn edit_provider(app_type: AppType, id: &str) -> Result<(), AppError> {
    // Disable bracketed paste mode to work around inquire dropping paste events
    crate::cli::terminal::disable_bracketed_paste_mode_best_effort();

    println!("{}", highlight(&format!("Edit Provider: {}", id)));
    println!("{}", "=".repeat(50));

    // 1. 加载并验证供应商存在
    let state = AppState::try_new()?;
    let config = state.config.read().unwrap();
    let manager = config
        .get_manager(&app_type)
        .ok_or_else(|| AppError::Message(texts::app_config_not_found(app_type.as_str())))?;
    let original = manager
        .providers
        .get(id)
        .ok_or_else(|| {
            let msg = texts::entity_not_found(texts::entity_provider(), id);
            AppError::localized("provider.not_found", msg.clone(), msg)
        })?
        .clone();
    let is_current = manager.current == id;
    let common_snippet = config.common_config_snippets.get(&app_type).cloned();
    drop(config);

    // 2. 显示当前配置
    println!("\n{}", highlight(texts::current_config_header()));
    display_provider_summary(&original, &app_type);
    println!();

    // 3. 全量编辑各字段（使用当前值作为默认）
    println!("{}", info(texts::edit_fields_instruction()));

    // 调用 prompt_basic_fields 来处理基本字段输入（自动使用 initial_value）
    let (name, website_url) = prompt_basic_fields(Some(&original))?;

    // 4. 询问是否修改配置
    let settings_config = if Confirm::new(texts::modify_provider_config_prompt())
        .with_default(false)
        .prompt()
        .map_err(|e| AppError::Message(texts::input_failed_error(&e.to_string())))?
    {
        prompt_settings_config(
            &app_type,
            Some(&original.settings_config),
            matches!(app_type, AppType::Codex) && is_codex_official_provider(&original),
        )?
    } else {
        original.settings_config.clone()
    };

    // 5. 询问是否修改可选字段
    let optional = if Confirm::new(texts::modify_optional_fields_prompt())
        .with_default(false)
        .prompt()
        .map_err(|e| AppError::Message(texts::input_failed_error(&e.to_string())))?
    {
        prompt_optional_fields(Some(&original))?
    } else {
        OptionalFields::from_provider(&original)
    };

    // 6. 构建更新后的 Provider（保留 meta 和 created_at）
    let mut updated = Provider {
        id: id.to_string(),
        name: name.trim().to_string(),
        settings_config,
        website_url,
        category: None,
        created_at: original.created_at,
        sort_index: optional.sort_index,
        notes: optional.notes,
        icon: None,
        icon_color: None,
        meta: original.meta,                           // 保留元数据
        in_failover_queue: original.in_failover_queue, // 保留故障转移状态
    };
    if let Some(enabled) =
        prompt_common_config_enabled(&app_type, common_snippet.as_deref(), Some(&updated))?
    {
        set_provider_common_config_meta(&mut updated, enabled);
    }

    // 7. 显示修改摘要并确认
    println!("\n{}", highlight(texts::updated_config_header()));
    display_provider_summary(&updated, &app_type);
    if !Confirm::new(&texts::confirm_update_entity(texts::entity_provider()))
        .with_default(false)
        .prompt()
        .map_err(|e| AppError::Message(texts::input_failed_error(&e.to_string())))?
    {
        println!("{}", info(texts::cancelled()));
        return Ok(());
    }

    // 8. 调用 Service 层
    ProviderService::update(&state, app_type.clone(), updated)?;

    // 9. 成功消息
    println!(
        "\n{}",
        success(&texts::entity_updated_success(texts::entity_provider(), id))
    );
    if is_current {
        println!("{}", warning(texts::current_provider_synced_warning()));
    }

    Ok(())
}

fn duplicate_provider(_app_type: AppType, id: &str) -> Result<(), AppError> {
    println!("{}", info(&format!("Duplicating provider '{}'...", id)));
    println!("{}", error("Provider duplication is not yet implemented."));
    Ok(())
}

fn export_provider(app_type: AppType, id: &str, output: Option<PathBuf>) -> Result<(), AppError> {
    if !matches!(app_type, AppType::Claude) {
        return Err(AppError::Message(format!(
            "Provider export currently supports only Claude standalone settings files. Use --app claude (current app: {}).",
            app_type.as_str()
        )));
    }

    let state = get_state()?;

    // Single lock scope: get provider AND common_config_snippet together
    let (provider, common_config_snippet) = {
        let config = state.config.read().map_err(AppError::from)?;
        let manager = config
            .get_manager(&app_type)
            .ok_or_else(|| AppError::Message(texts::app_config_not_found(app_type.as_str())))?;

        let provider = manager
            .providers
            .get(id)
            .ok_or_else(|| {
                let msg = texts::provider_not_found(id);
                AppError::localized("provider.not_found", msg.clone(), msg)
            })?
            .clone();

        (
            provider,
            config.common_config_snippets.get(&app_type).cloned(),
        )
    };

    let apply_common_config = ProviderService::provider_uses_common_config_for_app(
        &app_type,
        &provider,
        common_config_snippet.as_deref(),
    );

    let output_path = match output {
        None => {
            // Default: {cwd}/.claude/settings.local.json (auto-loaded by Claude CLI)
            std::env::current_dir()
                .map_err(|e| AppError::Message(format!("无法获取当前工作目录: {}", e)))?
                .join(".claude")
                .join("settings.local.json")
        }
        Some(path) => {
            // If path looks like a directory (no .json extension), append settings-{name}.json
            let path_str = path.to_string_lossy();
            if path_str.ends_with('/') || path_str.ends_with('\\') || !path_str.ends_with(".json") {
                path.join(format!(
                    "settings-{}.json",
                    crate::config::sanitize_provider_name(&provider.name)
                ))
            } else {
                path
            }
        }
    };

    if output_path.exists() {
        let confirm = Confirm::new(&format!(
            "File '{}' already exists. Overwrite?",
            output_path.display()
        ))
        .with_default(false)
        .prompt()
        .map_err(|e| AppError::Message(texts::input_failed_error(&e.to_string())))?;

        if !confirm {
            println!("{}", info(texts::cancelled()));
            return Ok(());
        }
    }

    let settings_content = ProviderService::build_live_backup_snapshot(
        &app_type,
        &provider,
        common_config_snippet.as_deref(),
        apply_common_config,
    )?;

    crate::config::write_json_file(&output_path, &settings_content)?;

    println!(
        "{}",
        success(&format!(
            "✓ Exported provider '{}' to {}",
            id,
            output_path.display()
        ))
    );

    // If output is settings.local.json, Claude CLI will auto-load it
    if output_path
        .file_name()
        .map(|n| n.to_string_lossy() == "settings.local.json")
        .unwrap_or(false)
    {
        println!(
            "{}",
            info("Claude CLI will auto-load this config. Just run: claude")
        );
    } else {
        println!(
            "{}",
            info(&format!(
                "Use it with: claude --settings {}",
                output_path.display()
            ))
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_codex_oauth_provider_uses_default_account_binding_shape() {
        let provider = build_codex_oauth_provider("codex", "Codex", None);
        let env = provider
            .settings_config
            .get("env")
            .and_then(|value| value.as_object())
            .expect("codex oauth provider should carry Claude env defaults");
        let meta = provider.meta.expect("codex oauth provider should have meta");
        let binding = meta.auth_binding.expect("codex oauth provider should bind auth");

        assert_eq!(provider.id, "codex");
        assert_eq!(provider.name, "Codex");
        assert_eq!(
            env.get("ANTHROPIC_BASE_URL")
                .and_then(|value| value.as_str()),
            Some("https://chatgpt.com/backend-api/codex")
        );
        assert_eq!(
            env.get("ANTHROPIC_MODEL")
                .and_then(|value| value.as_str()),
            Some("gpt-5.4")
        );
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
                .and_then(|value| value.as_str()),
            Some("gpt-5.4-mini")
        );
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                .and_then(|value| value.as_str()),
            Some("gpt-5.4")
        );
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_OPUS_MODEL")
                .and_then(|value| value.as_str()),
            Some("gpt-5.4")
        );
        assert_eq!(meta.provider_type.as_deref(), Some("codex_oauth"));
        assert_eq!(binding.source, crate::provider::AuthBindingSource::ManagedAccount);
        assert_eq!(binding.auth_provider.as_deref(), Some("codex_oauth"));
        assert_eq!(binding.account_id, None);
    }

    #[test]
    fn build_codex_oauth_provider_preserves_explicit_account_binding() {
        let provider = build_codex_oauth_provider("codex", "Codex", Some("acct_123"));
        let binding = provider
            .meta
            .expect("codex oauth provider should have meta")
            .auth_binding
            .expect("codex oauth provider should bind auth");

        assert_eq!(binding.account_id.as_deref(), Some("acct_123"));
    }
}
