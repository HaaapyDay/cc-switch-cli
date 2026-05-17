use clap::Subcommand;
use std::time::{Duration, Instant};

use crate::cli::ui::{highlight, info, success, warning};
use crate::error::AppError;
use crate::services::AuthService;

const AUTH_PROVIDER_CODEX_OAUTH: &str = "codex_oauth";
const AUTH_PROVIDER_CODEX_OAUTH_CLI: &str = "codex-oauth";

#[derive(Subcommand)]
pub enum AuthCommand {
    /// List supported managed auth providers
    Providers,
    /// Log in to a managed auth provider
    Login {
        /// Auth provider, for example codex-oauth
        #[arg(value_parser = [AUTH_PROVIDER_CODEX_OAUTH_CLI])]
        provider: String,
    },
    /// Show managed auth status
    Status {
        /// Auth provider, for example codex-oauth
        #[arg(value_parser = [AUTH_PROVIDER_CODEX_OAUTH_CLI])]
        provider: String,
    },
    /// List managed auth accounts
    Accounts {
        /// Auth provider, for example codex-oauth
        #[arg(value_parser = [AUTH_PROVIDER_CODEX_OAUTH_CLI])]
        provider: String,
    },
    /// Select the default managed auth account
    Use {
        /// Auth provider, for example codex-oauth
        #[arg(value_parser = [AUTH_PROVIDER_CODEX_OAUTH_CLI])]
        provider: String,
        /// Account ID to use by default
        account_id: String,
    },
    /// Remove one managed auth account
    Remove {
        /// Auth provider, for example codex-oauth
        #[arg(value_parser = [AUTH_PROVIDER_CODEX_OAUTH_CLI])]
        provider: String,
        /// Account ID to remove
        account_id: String,
    },
    /// Log out managed auth accounts
    Logout {
        /// Auth provider, for example codex-oauth
        #[arg(value_parser = [AUTH_PROVIDER_CODEX_OAUTH_CLI])]
        provider: String,
        /// Remove all accounts for this auth provider
        #[arg(long)]
        all: bool,
    },
}

pub fn execute(cmd: AuthCommand) -> Result<(), AppError> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| AppError::Message(format!("Failed to create async runtime: {error}")))?;

    match cmd {
        AuthCommand::Providers => list_providers(),
        AuthCommand::Login { provider } => {
            let provider = normalize_provider(&provider)?;
            runtime.block_on(login(&provider))
        }
        AuthCommand::Status { provider } => {
            let provider = normalize_provider(&provider)?;
            runtime.block_on(show_status(&provider))
        }
        AuthCommand::Accounts { provider } => {
            let provider = normalize_provider(&provider)?;
            runtime.block_on(list_accounts(&provider))
        }
        AuthCommand::Use {
            provider,
            account_id,
        } => {
            let provider = normalize_provider(&provider)?;
            runtime.block_on(use_account(&provider, &account_id))
        }
        AuthCommand::Remove {
            provider,
            account_id,
        } => {
            let provider = normalize_provider(&provider)?;
            runtime.block_on(remove_account(&provider, &account_id))
        }
        AuthCommand::Logout { provider, all } => {
            let provider = normalize_provider(&provider)?;
            runtime.block_on(logout(&provider, all))
        }
    }
}

fn normalize_provider(provider: &str) -> Result<String, AppError> {
    match provider.trim().to_ascii_lowercase().as_str() {
        AUTH_PROVIDER_CODEX_OAUTH | AUTH_PROVIDER_CODEX_OAUTH_CLI | "codexoauth" => {
            Ok(AUTH_PROVIDER_CODEX_OAUTH.to_string())
        }
        other => Err(AppError::Message(format!(
            "Unsupported auth provider: {other}. Supported providers: {AUTH_PROVIDER_CODEX_OAUTH_CLI}"
        ))),
    }
}

fn display_provider(provider: &str) -> &'static str {
    match provider {
        AUTH_PROVIDER_CODEX_OAUTH => AUTH_PROVIDER_CODEX_OAUTH_CLI,
        _ => AUTH_PROVIDER_CODEX_OAUTH_CLI,
    }
}

fn list_providers() -> Result<(), AppError> {
    println!("{}", highlight("Managed Auth Providers"));
    println!("  {}", AUTH_PROVIDER_CODEX_OAUTH_CLI);
    Ok(())
}

async fn login(provider: &str) -> Result<(), AppError> {
    let response = AuthService::start_login(provider)
        .await
        .map_err(AppError::Message)?;
    let provider_label = display_provider(provider);

    println!("{}", highlight(&format!("Login: {provider_label}")));
    println!(
        "{}",
        info("Open the verification URL and enter the user code:")
    );
    println!("  URL:  {}", response.verification_uri);
    println!("  Code: {}", response.user_code);
    println!();
    println!("{}", info("Waiting for authorization..."));

    let deadline = Instant::now() + Duration::from_secs(response.expires_in);
    let interval = Duration::from_secs(response.interval.max(1));

    loop {
        match AuthService::poll_for_account(provider, &response.device_code)
            .await
            .map_err(AppError::Message)?
        {
            Some(account) => {
                println!(
                    "{}",
                    success(&format!("Logged in as {} ({})", account.login, account.id))
                );
                return Ok(());
            }
            None => {
                if Instant::now() >= deadline {
                    return Err(AppError::Message(
                        "Login timed out before authorization completed".to_string(),
                    ));
                }
                tokio::time::sleep(interval).await;
            }
        }
    }
}

async fn show_status(provider: &str) -> Result<(), AppError> {
    let status = AuthService::get_status(provider)
        .await
        .map_err(AppError::Message)?;

    println!(
        "{}",
        highlight(&format!("Auth Status: {}", display_provider(provider)))
    );
    println!("  Authenticated: {}", status.authenticated);
    println!(
        "  Default Account: {}",
        status.default_account_id.as_deref().unwrap_or("-")
    );
    println!("  Accounts: {}", status.accounts.len());

    Ok(())
}

async fn list_accounts(provider: &str) -> Result<(), AppError> {
    let accounts = AuthService::list_accounts(provider)
        .await
        .map_err(AppError::Message)?;

    println!(
        "{}",
        highlight(&format!("Accounts: {}", display_provider(provider)))
    );
    if accounts.is_empty() {
        println!(
            "{}",
            warning("No accounts found. Run: cc-switch auth login codex-oauth")
        );
        return Ok(());
    }

    for account in accounts {
        let marker = if account.is_default { "*" } else { " " };
        println!("{marker} {}  {}", account.id, account.login);
    }

    Ok(())
}

async fn use_account(provider: &str, account_id: &str) -> Result<(), AppError> {
    AuthService::set_default_account(provider, account_id)
        .await
        .map_err(AppError::Message)?;
    println!(
        "{}",
        success(&format!("Default account set to {account_id}"))
    );
    Ok(())
}

async fn remove_account(provider: &str, account_id: &str) -> Result<(), AppError> {
    AuthService::remove_account(provider, account_id)
        .await
        .map_err(AppError::Message)?;
    println!("{}", success(&format!("Removed account {account_id}")));
    Ok(())
}

async fn logout(provider: &str, all: bool) -> Result<(), AppError> {
    if !all {
        return Err(AppError::Message(
            "Refusing to remove all accounts without --all".to_string(),
        ));
    }

    AuthService::logout(provider)
        .await
        .map_err(AppError::Message)?;
    println!(
        "{}",
        success(&format!("Logged out {}", display_provider(provider)))
    );
    Ok(())
}
