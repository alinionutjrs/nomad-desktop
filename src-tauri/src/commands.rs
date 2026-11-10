use crate::agent_tools;
use crate::cli_agents::{CliAgent, CliAgentProvider, CliToolStatus};
use crate::confirmations::ConfirmationManager;
use crate::db::Database;
use crate::git;
use crate::llm::{LlmMessage, LlmProvider, LlmRequest, OpenAiProvider};
use crate::models::*;
use crate::process_manager::ProcessManager;
use crate::security::Workspace;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

pub struct AppState {
    pub db: Arc<Database>,
    pub confirmations: Arc<ConfirmationManager>,
    pub processes: Arc<ProcessManager>,
    pub chat_cancellations:
        Arc<AsyncMutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<()>>>>,
}

type CommandResult<T> = std::result::Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInfo {
    version: String,
    current_version: String,
    notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomAiProvider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    #[serde(default = "default_ai_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub api_key_env: String,
}

fn default_ai_protocol() -> String {
    "openai-compatible".to_string()
}

fn custom_provider_credential_id(id: &str) -> Result<String> {
    if id.is_empty()
        || id.len() > 40
        || !id.as_bytes()[0].is_ascii_lowercase() && !id.as_bytes()[0].is_ascii_digit()
        || !id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
    {
        bail!("ID-ul providerului trebuie să conțină doar litere mici, cifre, cratimă sau underscore (maximum 40 de caractere)");
    }
    Ok(format!("custom-provider-{id}"))
}

fn load_custom_ai_providers(db: &Database) -> Result<Vec<CustomAiProvider>> {
    db.get_setting(0, "custom_ai_providers")?
        .filter(|raw| !raw.trim().is_empty())
        .map(|raw| serde_json::from_str(&raw).context("Lista providerelor custom este invalidă"))
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn load_custom_ai_provider(db: &Database, id: &str) -> Result<CustomAiProvider> {
    load_custom_ai_providers(db)?
        .into_iter()
        .find(|provider| provider.id == id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Providerul custom selectat nu mai există. Alege alt provider în Setări AI."
            )
        })
}

const UPDATER_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEU1MEVCRTA1NTI3MzQ5MDAKUldRQVNYTlNCYjRPNVNQUGZGaFdjZTVqN2lxWmxCVEp3TmllQVdacy85ZXNqdkRlaHUwMDBJQnkK";
const DEFAULT_UPDATE_ENDPOINT: Option<&str> = option_env!("NOMAD_UPDATE_ENDPOINT");

fn update_endpoint(state: &AppState) -> Result<url::Url> {
    let raw = state
        .db
        .get_setting(0, "update_endpoint")?
        .filter(|value| !value.trim().is_empty())
        .or_else(|| DEFAULT_UPDATE_ENDPOINT.map(str::to_string))
        .unwrap_or_default();
    if raw.trim().is_empty() {
        bail!("Adaugă feed-ul HTTPS al release-urilor Nomad în Setări → Actualizări.");
    }
    let endpoint = url::Url::parse(raw.trim()).context("Feed-ul de update nu este un URL valid")?;
    if endpoint.scheme() != "https" || endpoint.host_str().is_none() {
        bail!("Feed-ul de update trebuie să folosească HTTPS.");
    }
    Ok(endpoint)
}

#[tauri::command]
pub async fn check_for_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Option<AppUpdateInfo>> {
    async {
        let endpoint = update_endpoint(&state)?;
        let update = app
            .updater_builder()
            .pubkey(UPDATER_PUBLIC_KEY)
            .endpoints(vec![endpoint])?
            .build()?
            .check()
            .await?;
        Ok(update.map(|update| AppUpdateInfo {
            version: update.version,
            current_version: update.current_version,
            notes: update.body,
        }))
    }
    .await
    .map_err(|error: anyhow::Error| error.to_string())
}

#[tauri::command]
pub async fn install_app_update(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    let result: Result<()> = async {
        let endpoint = update_endpoint(&state)?;
        let update = app
            .updater_builder()
            .pubkey(UPDATER_PUBLIC_KEY)
            .endpoints(vec![endpoint])?
            .build()?
            .check()
            .await?
            .ok_or_else(|| anyhow::anyhow!("Nu există o versiune nouă disponibilă"))?;
        update.download_and_install(|_, _| {}, || {}).await?;
        Ok(())
    }
    .await;
    result.map_err(|error| error.to_string())?;
    app.restart()
}

fn command<T>(value: Result<T>) -> CommandResult<T> {
    value.map_err(|error| error.to_string())
}

fn workspace(db: &Database, project_id: i64) -> Result<Workspace> {
    let project = db.get_project(project_id)?;
    Workspace::new(&project.path)
}

fn valid_project_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 80 {
        bail!("Numele proiectului trebuie să aibă 1–80 caractere");
    }
    if name == "." || name == ".." || name.chars().any(|ch| "<>:\"/\\|?*".contains(ch)) {
        bail!("Numele proiectului conține caractere nepermise");
    }
    Ok(())
}

fn validate_local_url(value: &str) -> Result<String> {
    let mut url = url::Url::parse(value).context("Adresa preview nu este validă")?;
    let host = url.host_str().unwrap_or_default().to_string();
    if !matches!(url.scheme(), "http" | "https")
        || !(host == "localhost" || host == "127.0.0.1" || host == "[::1]")
    {
        bail!("Preview-ul trebuie să folosească localhost sau 127.0.0.1");
    }
    if host == "localhost" {
        url.set_host(Some("127.0.0.1"))
            .map_err(|_| anyhow::anyhow!("Adresa localhost nu poate fi normalizată"))?;
    }
    Ok(url.to_string())
}

fn display_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
        if let Some(local) = raw.strip_prefix(r"\\?\") {
            return local.to_string();
        }
    }
    raw.into_owned()
}

fn is_inside_repository(root: &Path) -> Result<bool> {
    let mut current = root.parent();
    while let Some(directory) = current {
        if directory.join(".git").exists() {
            return Ok(true);
        }
        current = directory.parent();
    }
    Ok(false)
}

fn starter_files(root: &Path, name: &str) -> Result<()> {
    fs::create_dir_all(root.join("src"))?;
    fs::write(
        root.join("package.json"),
        format!(
            r#"{{
  "name": "{}",
  "private": true,
  "scripts": {{ "dev": "node dev-server.mjs" }}
}}
"#,
            name.to_lowercase().replace(' ', "-")
        ),
    )?;
    fs::write(
        root.join("index.html"),
        r#"<!doctype html>
<html lang="ro">
  <head><meta charset="UTF-8"><meta name="viewport" content="width=device-width"><title>Nomad App</title><link rel="stylesheet" href="/src/styles.css"></head>
  <body><main><span>CREAT CU NOMAD</span><h1>Primul tău proiect local-first.</h1><p>Deschide fișierul <code>index.html</code>, cere o schimbare și aprobă diff-ul.</p></main></body>
</html>
"#,
    )?;
    fs::write(
        root.join("src").join("styles.css"),
        r#":root{font-family:system-ui;color:#eef3e8;background:#10140e}body{margin:0;min-height:100vh;display:grid;place-items:center}main{max-width:680px;padding:50px}span{color:#b7f36b;font-size:12px;font-weight:800;letter-spacing:.16em}h1{font-size:clamp(42px,8vw,78px);line-height:.95;letter-spacing:-.06em;margin:24px 0}p{color:#9da895;font-size:18px;line-height:1.6}code{color:#d6f5b0}
"#,
    )?;
    fs::write(
        root.join("dev-server.mjs"),
        r#"import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, isAbsolute, relative, resolve } from 'node:path';
const root = process.cwd();
const types = {'.html':'text/html; charset=utf-8','.css':'text/css; charset=utf-8','.js':'text/javascript; charset=utf-8','.json':'application/json'};
createServer(async (req,res) => {
  try {
    const urlPath = decodeURIComponent((req.url || '/').split('?')[0]);
    const requestPath = urlPath === '/' ? 'index.html' : urlPath.replace(/^\/+/, '');
    const file = resolve(root, requestPath);
    const withinRoot = relative(root, file);
    if (withinRoot.startsWith('..') || isAbsolute(withinRoot)) throw new Error('forbidden');
    const info = await stat(file);
    if (!info.isFile()) throw new Error('not found');
    res.writeHead(200, {'content-type': types[extname(file)] || 'application/octet-stream', 'cache-control':'no-store'});
    res.end(await readFile(file));
  } catch { res.writeHead(404); res.end('Not found'); }
}).listen(5173, '127.0.0.1', () => console.log('Preview: http://localhost:5173'));
"#,
    )?;
    Ok(())
}

fn ensure_starter_settings(db: &Database, project_id: i64) -> Result<()> {
    for (key, value) in [
        ("dev_command", "node dev-server.mjs"),
        ("preview_url", "http://127.0.0.1:5173"),
    ] {
        if db.get_setting(project_id, key)?.is_none() {
            db.set_setting(project_id, key, value)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn request_confirmation(
    action: String,
    description: String,
    state: State<'_, AppState>,
) -> CommandResult<ConfirmationChallenge> {
    command(state.confirmations.request(action, description))
}

#[tauri::command]
pub fn cancel_confirmation(
    confirmation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    command(state.confirmations.cancel(&confirmation_id))
}

#[tauri::command]
pub fn confirm_action(
    confirmation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    command(state.confirmations.approve(&confirmation_id))
}

#[tauri::command]
pub fn create_project(
    parent_dir: String,
    name: String,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<Project> {
    command((|| {
        state.confirmations.consume(
            &confirmation_token,
            &format!("project:create:{parent_dir}:{name}"),
        )?;
        valid_project_name(&name)?;
        let parent = fs::canonicalize(&parent_dir).context("Directorul părinte nu există")?;
        if !parent.is_dir() {
            bail!("Calea părinte nu este director");
        }
        if is_inside_repository(&parent)? {
            bail!("Alege un director din afara unui repository Git existent pentru proiectul nou.");
        }
        let root = parent.join(&name);
        if root.exists() {
            bail!("Directorul proiectului există deja");
        }
        fs::create_dir(&root)?;
        starter_files(&root, &name)?;
        let canonical = fs::canonicalize(&root)?;
        let project = state.db.upsert_project(&name, &display_path(&canonical))?;
        ensure_starter_settings(&state.db, project.id)?;
        Ok(project)
    })())
}

#[tauri::command]
pub fn open_project(path: String, state: State<'_, AppState>) -> CommandResult<Project> {
    command((|| {
        let root = fs::canonicalize(&path).context("Directorul nu există")?;
        if !root.is_dir() {
            bail!("Calea nu este un director");
        }
        let workspace = Workspace::new(&root)?;
        let entries = agent_tools::list_files(&workspace)?;
        if entries.is_empty() {
            let name = root
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Proiect");
            starter_files(&root, name)?;
        }
        let name = root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Proiect");
        let project = state.db.upsert_project(name, &display_path(&root))?;
        if root.join("dev-server.mjs").is_file() {
            ensure_starter_settings(&state.db, project.id)?;
        }
        Ok(project)
    })())
}

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> CommandResult<Vec<Project>> {
    command(state.db.list_projects())
}

fn copy_export_tree(source: &Path, target: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in walkdir::WalkDir::new(source)
        .follow_links(false)
        .min_depth(1)
        .max_depth(32)
        .into_iter()
        .filter_entry(|entry| {
            !(entry.file_type().is_dir()
                && matches!(
                    entry.file_name().to_string_lossy().as_ref(),
                    ".git"
                        | "node_modules"
                        | "target"
                        | "dist"
                        | ".next"
                        | ".turbo"
                        | ".venv"
                        | "venv"
                ))
        })
    {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            continue;
        }
        let relative = entry.path().strip_prefix(source)?;
        let portable = relative.to_string_lossy().replace('\\', "/");
        if crate::agent_tools::is_sensitive_path(&portable) {
            continue;
        }
        let output = target.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(output)?;
        } else if entry.file_type().is_file() {
            if count >= 100_000 {
                bail!("Exportul depășește limita de 100.000 de fișiere.");
            }
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), output)?;
            count += 1;
        }
    }
    Ok(count)
}

#[tauri::command]
pub fn export_project_files(
    project_id: i64,
    destination_parent: String,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    command((|| {
        state.confirmations.consume(
            &confirmation_token,
            &format!("project:export:{project_id}:{destination_parent}"),
        )?;
        let project = state.db.get_project(project_id)?;
        let source = fs::canonicalize(&project.path).context("Directorul proiectului nu există")?;
        let parent = fs::canonicalize(&destination_parent)
            .context("Directorul ales pentru export nu există")?;
        if !parent.is_dir() || parent.starts_with(&source) {
            bail!("Alege un director de export din afara proiectului.");
        }
        let folder_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("nomad-project");
        let target = parent.join(format!("{folder_name}-export"));
        if target.exists() {
            bail!("Directorul de export există deja: {}", target.display());
        }
        fs::create_dir(&target)?;
        let copied = copy_export_tree(&source, &target);
        match copied {
            Ok(count) => Ok(format!("{}|{}", target.display(), count)),
            Err(error) => {
                let _ = fs::remove_dir_all(&target);
                Err(error)
            }
        }
    })())
}

#[tauri::command]
pub fn list_project_files(
    project_id: i64,
    state: State<'_, AppState>,
) -> CommandResult<Vec<FileEntry>> {
    command(workspace(&state.db, project_id).and_then(|value| agent_tools::list_files(&value)))
}

#[tauri::command]
pub fn read_file(
    project_id: i64,
    path: String,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    command(
        workspace(&state.db, project_id).and_then(|value| agent_tools::read_file(&value, &path)),
    )
}

#[tauri::command]
pub fn search_files(
    project_id: i64,
    query: String,
    state: State<'_, AppState>,
) -> CommandResult<Vec<SearchMatch>> {
    command(
        workspace(&state.db, project_id)
            .and_then(|value| agent_tools::search_files(&value, &query)),
    )
}

#[tauri::command]
pub fn write_file(
    project_id: i64,
    path: String,
    content: String,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    command((|| {
        state.confirmations.consume(
            &confirmation_token,
            &format!("file:write:{project_id}:{path}"),
        )?;
        agent_tools::write_file(&workspace(&state.db, project_id)?, &path, &content)
    })())
}

#[tauri::command]
pub fn apply_patch(
    project_id: i64,
    path: String,
    expected: String,
    replacement: String,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    command((|| {
        state.confirmations.consume(
            &confirmation_token,
            &format!("file:patch:{project_id}:{path}"),
        )?;
        agent_tools::apply_patch(
            &workspace(&state.db, project_id)?,
            &path,
            &expected,
            &replacement,
        )
    })())
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn preview_accepts_only_loopback_urls() {
        assert_eq!(
            validate_local_url("http://localhost:5173").unwrap(),
            "http://127.0.0.1:5173/"
        );
        assert_eq!(
            validate_local_url("http://127.0.0.1:8080/path").unwrap(),
            "http://127.0.0.1:8080/path"
        );
        assert!(validate_local_url("https://example.com").is_err());
        assert!(validate_local_url("file:///C:/secret").is_err());
    }

    #[test]
    fn settings_validate_values_and_reject_unknown_keys() {
        assert!(validate_app_setting("workflow_instructions", "Propune schimbări mici").is_ok());
        assert!(validate_app_setting("workflow_timeout_seconds", "120").is_ok());
        assert!(validate_app_setting("workflow_timeout_seconds", "99999").is_err());
        assert!(validate_app_setting("mcp_servers_json", "[]").is_ok());
        assert!(validate_app_setting("mcp_servers_json", "{}").is_err());
        assert!(validate_app_setting("confirm_file_writes", "false").is_ok());
        assert!(validate_app_setting("confirm_file_writes", "yes").is_err());
        assert!(validate_app_setting("unknown_setting", "value").is_err());
    }
}

#[tauri::command]
pub async fn run_command(
    project_id: i64,
    program: String,
    args: Vec<String>,
    timeout_seconds: u64,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<CommandOutput> {
    let action = format!(
        "command:run:{project_id}:{program}:{}",
        serde_json::to_string(&args).map_err(|error| error.to_string())?
    );
    if let Err(error) = state.confirmations.consume(&confirmation_token, &action) {
        return Err(error.to_string());
    }
    let workspace = match workspace(&state.db, project_id) {
        Ok(value) => value,
        Err(error) => return Err(error.to_string()),
    };
    let configured_timeout = state
        .db
        .get_setting(0, "tool_command_timeout")
        .map_err(|error| error.to_string())?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);
    let effective_timeout = timeout_seconds.min(configured_timeout).clamp(5, 3600);
    command(agent_tools::run_command(&workspace, &program, &args, effective_timeout).await)
}

#[tauri::command]
pub fn git_diff(project_id: i64, state: State<'_, AppState>) -> CommandResult<String> {
    command(workspace(&state.db, project_id).and_then(|value| git::diff(value.root())))
}

#[tauri::command]
pub fn list_messages(
    project_id: i64,
    state: State<'_, AppState>,
) -> CommandResult<Vec<ChatMessage>> {
    command((|| {
        state.db.get_project(project_id)?;
        state.db.list_messages(project_id, 100)
    })())
}

#[tauri::command]
pub fn list_session_messages(
    session_id: i64,
    state: State<'_, AppState>,
) -> CommandResult<Vec<ChatMessage>> {
    command(state.db.list_session_messages(session_id, 100))
}

#[tauri::command]
pub fn list_proposals(project_id: i64, state: State<'_, AppState>) -> CommandResult<Vec<Proposal>> {
    command((|| {
        state.db.get_project(project_id)?;
        state.db.list_proposals(project_id)
    })())
}

#[tauri::command]
pub fn create_manual_proposal(
    project_id: i64,
    path: String,
    content: String,
    summary: String,
    expected_original: String,
    state: State<'_, AppState>,
) -> CommandResult<Proposal> {
    command((|| {
        let workspace = workspace(&state.db, project_id)?;
        let target = workspace.resolve_for_write(&path)?;
        let original = if target.exists() {
            agent_tools::read_file(&workspace, &path)?
        } else {
            String::new()
        };
        if original != expected_original {
            bail!("Fișierul s-a schimbat pe disc. Reîncarcă-l înainte să propui modificarea.");
        }
        state
            .db
            .create_proposal(project_id, &path, &original, &content, &summary)
    })())
}

#[tauri::command]
pub fn reject_proposal(proposal_id: i64, state: State<'_, AppState>) -> CommandResult<Proposal> {
    command((|| {
        let proposal = state.db.get_proposal(proposal_id)?;
        let _ = workspace(&state.db, proposal.project_id)?;
        if proposal.status != "pending" {
            bail!("Propunerea nu mai este în așteptarea unei decizii");
        }
        state.db.update_proposal_status(proposal_id, "rejected")
    })())
}

#[tauri::command]
pub fn accept_proposal(
    project_id: i64,
    proposal_id: i64,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<Proposal> {
    command((|| {
        state.confirmations.consume(
            &confirmation_token,
            &format!("proposal:accept:{proposal_id}"),
        )?;
        let proposal = state.db.get_proposal(proposal_id)?;
        if proposal.project_id != project_id || proposal.status != "pending" {
            bail!("Propunerea nu poate fi aplicată");
        }
        let workspace = workspace(&state.db, project_id)?;
        let target = workspace.resolve_for_write(&proposal.path)?;
        let current = if target.exists() {
            agent_tools::read_file(&workspace, &proposal.path)?
        } else {
            String::new()
        };
        if current != proposal.original_content {
            bail!("Fișierul s-a schimbat după crearea propunerii. Creează un diff nou.");
        }
        git::create_snapshot(workspace.root(), "Nomad: înainte de modificare")?;
        agent_tools::write_file(&workspace, &proposal.path, &proposal.proposed_content)?;
        git::create_snapshot(workspace.root(), &format!("Nomad: {}", proposal.summary))?;
        state.db.update_proposal_status(proposal_id, "accepted")
    })())
}

fn extract_change(text: &str) -> Result<Option<ProposedChange>> {
    let Some(start) = text.find("<nomad-change>") else {
        return Ok(None);
    };
    let json_start = start + "<nomad-change>".len();
    let end = text[json_start..]
        .find("</nomad-change>")
        .map(|value| json_start + value)
        .ok_or_else(|| anyhow::anyhow!("Răspunsul AI conține o propunere incompletă"))?;
    Ok(Some(
        serde_json::from_str(&text[json_start..end]).context("Propunerea AI nu este JSON valid")?,
    ))
}

#[tauri::command]
pub fn chat_stream(
    project_id: i64,
    session_id: i64,
    prompt: String,
    active_path: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    if prompt.trim().is_empty() {
        return Err("Mesajul este gol".into());
    }
    if prompt.len() > 40_000 {
        return Err("Mesajul depășește limita de 40 KB".into());
    }
    let provider_kind = state
        .db
        .get_setting(0, "ai_provider")
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "api".into());
    let mut custom_model = None;
    let provider: Box<dyn LlmProvider> = match provider_kind.as_str() {
        "claude-code" => Box::new(CliAgentProvider(CliAgent::ClaudeCode)),
        "codex" => Box::new(CliAgentProvider(CliAgent::Codex)),
        "api" => {
            let endpoint = state
                .db
                .get_setting(0, "ai_endpoint")
                .map_err(|error| error.to_string())?;
            Box::new(
                OpenAiProvider::from_keychain(
                    endpoint.as_deref(),
                    "openai-api-key",
                    None,
                    "openai-compatible",
                )
                .map_err(|error| error.to_string())?,
            )
        }
        "custom" => {
            let selected_id = state
                .db
                .get_setting(0, "active_custom_ai_provider")
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Selectează un provider custom în Setări AI.".to_string())?;
            let custom = load_custom_ai_provider(&state.db, &selected_id)
                .map_err(|error| error.to_string())?;
            let credential_id =
                custom_provider_credential_id(&custom.id).map_err(|error| error.to_string())?;
            custom_model = Some(custom.model.clone());
            Box::new(
                OpenAiProvider::from_keychain(
                    Some(&custom.base_url),
                    &credential_id,
                    (!custom.api_key_env.trim().is_empty()).then_some(custom.api_key_env.as_str()),
                    &custom.protocol,
                )
                .map_err(|error| error.to_string())?,
            )
        }
        _ => return Err("Providerul AI selectat nu este valid".into()),
    };
    let workspace = workspace(&state.db, project_id).map_err(|error| error.to_string())?;
    let (session_project, _) = state
        .db
        .get_chat_session(session_id)
        .map_err(|error| error.to_string())?;
    if session_project != project_id {
        return Err("Sesiunea nu aparține proiectului".into());
    }
    state
        .db
        .insert_session_message(session_id, "user", prompt.trim())
        .map_err(|error| error.to_string())?;
    let history = state
        .db
        .list_session_messages(session_id, 16)
        .map_err(|error| error.to_string())?;
    let files = agent_tools::list_files(&workspace).map_err(|error| error.to_string())?;
    if active_path
        .as_deref()
        .is_some_and(agent_tools::is_sensitive_path)
    {
        return Err(
            "Fișierele cu secrete sau credențiale nu pot fi trimise unui provider AI".into(),
        );
    }
    let file_context = active_path
        .as_deref()
        .map(|path| {
            agent_tools::read_file(&workspace, path).map(|content| {
                format!(
                    "\nFișier activ: {path}\n```\n{}\n```",
                    content.chars().take(60_000).collect::<String>()
                )
            })
        })
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let project_map = files
        .iter()
        .take(250)
        .filter(|entry| !agent_tools::is_sensitive_path(&entry.path))
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let model = custom_model.unwrap_or(
        state
            .db
            .get_setting(0, "openai_model")
            .map_err(|error| error.to_string())?
            .unwrap_or_else(|| "gpt-6-astra".into()),
    );
    let workflow_instructions = state
        .db
        .get_setting(0, "workflow_instructions")
        .map_err(|error| error.to_string())?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_default();
    let instructions = format!(
        r#"Ești agentul de programare din Nomad, un mediu local-first. Răspunde concis în limba utilizatorului.
În acest MVP poți analiza contextul și propune cel mult o schimbare de fișier. Nu pretinde că ai executat unelte sau comenzi.
Când este necesară o modificare, încheie răspunsul cu exact un bloc pe o singură linie:
<nomad-change>{{"path":"cale/relativă","summary":"rezumat scurt","content":"conținutul COMPLET al fișierului"}}</nomad-change>
Calea trebuie să fie relativă la proiect. Nu folosi .., căi absolute sau symlink-uri. Nu include blocul dacă utilizatorul cere doar explicații.
Uneltele backend disponibile după confirmarea utilizatorului sunt: {}.

Instrucțiuni personale ale utilizatorului pentru acest agent:
{}

Fișiere proiect:
{}{}
"#,
        agent_tools::TOOL_NAMES.join(", "),
        workflow_instructions,
        project_map,
        file_context
    );
    let request = LlmRequest {
        model,
        instructions,
        messages: history
            .into_iter()
            .map(|message| LlmMessage {
                role: message.role,
                content: message.content,
            })
            .collect(),
        workspace: Some(workspace.root().to_path_buf()),
        max_output_tokens: None,
    };
    let request_id = Uuid::new_v4().to_string();
    let returned_id = request_id.clone();
    let db = state.db.clone();
    let app_handle = app.clone();
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    state
        .chat_cancellations
        .blocking_lock()
        .insert(request_id.clone(), cancel_tx);
    let cancellations = state.chat_cancellations.clone();
    tauri::async_runtime::spawn(async move {
        let (delta_tx, mut delta_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let delta_app = app_handle.clone();
        let delta_id = request_id.clone();
        let forwarder = tauri::async_runtime::spawn(async move {
            while let Some(delta) = delta_rx.recv().await {
                let _ = delta_app.emit(
                    "chat://delta",
                    StreamDelta {
                        request_id: delta_id.clone(),
                        delta,
                    },
                );
            }
        });
        let response = tokio::select! {
            _ = &mut cancel_rx => {
                let _ = app_handle.emit("chat://cancelled", StreamCancelled { request_id: request_id.clone() });
                cancellations.lock().await.remove(&request_id);
                return;
            }
            result = provider.stream(request, delta_tx) => result,
        };
        let _ = forwarder.await;
        match response {
            Ok(content) => {
                let result = (|| -> Result<(ChatMessage, Option<Proposal>)> {
                    let proposal = if let Some(change) = extract_change(&content)? {
                        let target = workspace.resolve_for_write(&change.path)?;
                        let original = if target.exists() {
                            agent_tools::read_file(&workspace, &change.path)?
                        } else {
                            String::new()
                        };
                        Some(db.create_proposal(
                            project_id,
                            &change.path,
                            &original,
                            &change.content,
                            &change.summary,
                        )?)
                    } else {
                        None
                    };
                    let message = db.insert_session_message(session_id, "assistant", &content)?;
                    Ok((message, proposal))
                })();
                match result {
                    Ok((message, proposal)) => {
                        let _ = app_handle.emit(
                            "chat://done",
                            StreamDone {
                                request_id: request_id.clone(),
                                message,
                                proposal,
                            },
                        );
                    }
                    Err(error) => {
                        let _ = app_handle.emit(
                            "chat://error",
                            StreamError {
                                request_id: request_id.clone(),
                                message: error.to_string(),
                            },
                        );
                    }
                }
            }
            Err(error) => {
                let _ = app_handle.emit(
                    "chat://error",
                    StreamError {
                        request_id: request_id.clone(),
                        message: error.to_string(),
                    },
                );
            }
        }
        cancellations.lock().await.remove(&request_id);
    });
    Ok(returned_id)
}

#[tauri::command]
pub fn list_chat_sessions(
    project_id: i64,
    state: State<'_, AppState>,
) -> CommandResult<Vec<serde_json::Value>> {
    command((|| {
        state.db.get_project(project_id)?;
        state.db.list_chat_sessions(project_id)
    })())
}

#[tauri::command]
pub fn create_chat_session(
    project_id: i64,
    title: String,
    state: State<'_, AppState>,
) -> CommandResult<i64> {
    command((|| {
        state.db.get_project(project_id)?;
        Ok(state.db.create_chat_session(project_id, &title)?)
    })())
}

#[tauri::command]
pub fn get_latest_session_id(
    project_id: i64,
    state: State<'_, AppState>,
) -> CommandResult<Option<i64>> {
    command((|| {
        state.db.get_project(project_id)?;
        state.db.latest_chat_session_id(project_id)
    })())
}

#[tauri::command]
pub async fn cancel_chat(request_id: String, state: State<'_, AppState>) -> CommandResult<()> {
    let sender = state.chat_cancellations.lock().await.remove(&request_id);
    if let Some(sender) = sender {
        let _ = sender.send(());
    }
    Ok(())
}

#[tauri::command]
pub fn save_api_key(api_key: String) -> CommandResult<()> {
    command(OpenAiProvider::save_key(&api_key))
}

#[tauri::command]
pub fn delete_api_key() -> CommandResult<()> {
    command(OpenAiProvider::delete_key())
}

#[tauri::command]
pub fn has_api_key() -> bool {
    OpenAiProvider::has_key()
}

#[tauri::command]
pub fn save_custom_api_key(provider_id: String, api_key: String) -> CommandResult<()> {
    command((|| {
        let credential_id = custom_provider_credential_id(&provider_id)?;
        OpenAiProvider::save_key_for(&credential_id, &api_key)
    })())
}

#[tauri::command]
pub fn delete_custom_api_key(provider_id: String) -> CommandResult<()> {
    command((|| {
        let credential_id = custom_provider_credential_id(&provider_id)?;
        OpenAiProvider::delete_key_for(&credential_id)
    })())
}

#[tauri::command]
pub fn has_custom_api_key(provider_id: String) -> bool {
    custom_provider_credential_id(&provider_id)
        .map(|credential_id| OpenAiProvider::has_key_for(&credential_id))
        .unwrap_or(false)
}

#[tauri::command]
pub async fn test_custom_ai_provider(
    provider_id: String,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    let result = async {
        let custom = load_custom_ai_provider(&state.db, &provider_id)?;
        let credential_id = custom_provider_credential_id(&custom.id)?;
        let provider = OpenAiProvider::from_keychain(
            Some(&custom.base_url),
            &credential_id,
            (!custom.api_key_env.trim().is_empty()).then_some(custom.api_key_env.as_str()),
            &custom.protocol,
        )?;
        let request = LlmRequest {
            model: custom.model,
            instructions: "You are checking an API connection. Reply with OK.".into(),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: "Connection test. Reply with OK.".into(),
            }],
            workspace: None,
            max_output_tokens: Some(8),
        };
        let (deltas, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            provider.stream(request, deltas),
        )
        .await
        .context("Testul providerului a depășit limita de 30 de secunde")??;
        if response.trim().is_empty() {
            bail!("Providerul s-a conectat, dar nu a returnat text");
        }
        Ok::<_, anyhow::Error>("Conexiunea a reușit. Providerul a returnat un răspuns.".to_string())
    }
    .await;
    command(result)
}

#[tauri::command]
pub fn detect_ai_cli_tools() -> Vec<CliToolStatus> {
    crate::cli_agents::statuses()
}

const APP_SETTING_KEYS: &[&str] = &[
    "openai_model",
    "ai_endpoint",
    "ai_provider",
    "active_custom_ai_provider",
    "custom_ai_providers",
    "theme",
    "ui_density",
    "profile_name",
    "update_endpoint",
    "anonymous_telemetry",
    "language",
    "workflow_instructions",
    "workflow_timeout_seconds",
    "workflow_auto_snapshot",
    "workflow_run_checks",
    "mcp_servers_json",
    "tool_command_timeout",
    "mcp_enabled",
    "mcp_network_access",
    "cloud_api_url",
    "deploy_provider",
    "deploy_project_id",
    "cloud_sync_enabled",
    "allowed_command_prefixes",
    "confirm_file_writes",
    "confirm_shell_commands",
    "allow_network_tools",
    "default_dev_command",
    "default_preview_url",
    "external_database_host",
    "verbose_logs",
    "experiment_notes",
    "experiment_multi_agent",
    "experiment_security_review",
    "experiment_templates",
    "support_contact",
    "support_notes",
    "attach_logs_to_support",
];

fn validate_app_setting(key: &str, value: &str) -> Result<()> {
    if !APP_SETTING_KEYS.contains(&key) {
        bail!("Setarea aplicației nu este permisă");
    }
    if value.len() > 64 * 1024 {
        bail!("Valoarea setării este prea lungă (maximum 64 KB)");
    }
    let boolean_setting = matches!(
        key,
        "anonymous_telemetry"
            | "workflow_auto_snapshot"
            | "workflow_run_checks"
            | "mcp_enabled"
            | "mcp_network_access"
            | "cloud_sync_enabled"
            | "confirm_file_writes"
            | "confirm_shell_commands"
            | "allow_network_tools"
            | "verbose_logs"
            | "experiment_multi_agent"
            | "experiment_security_review"
            | "experiment_templates"
            | "attach_logs_to_support"
    );
    if boolean_setting && !matches!(value, "true" | "false") {
        bail!("Valoarea acestei setări trebuie să fie true sau false");
    }
    match key {
        "openai_model" if value.trim().is_empty() || value.len() > 100 => {
            bail!("Modelul trebuie să aibă 1–100 caractere");
        }
        "theme" if !matches!(value, "forest" | "ocean" | "violet") => {
            bail!("Tema selectată nu este permisă");
        }
        "ui_density" if !matches!(value, "comfortable" | "compact") => {
            bail!("Densitatea selectată nu este permisă");
        }
        "ai_endpoint" if !value.trim().is_empty() => {
            crate::llm::chat_completions_endpoint(value.trim())?;
        }
        "ai_provider" if !matches!(value, "api" | "custom" | "claude-code" | "codex") => {
            bail!("Providerul AI selectat nu este permis");
        }
        "active_custom_ai_provider" if !value.trim().is_empty() => {
            custom_provider_credential_id(value.trim())?;
        }
        "profile_name" if value.trim().is_empty() || value.len() > 80 => {
            bail!("Numele profilului trebuie să aibă 1–80 caractere");
        }
        "update_endpoint" if !value.trim().is_empty() => {
            let endpoint =
                url::Url::parse(value.trim()).context("Feed-ul de update nu este un URL valid")?;
            if endpoint.scheme() != "https" || endpoint.host_str().is_none() {
                bail!("Feed-ul de update trebuie să folosească HTTPS.");
            }
        }
        "cloud_api_url" if !value.trim().is_empty() => {
            let endpoint =
                url::Url::parse(value.trim()).context("Adresa API cloud nu este validă")?;
            let host = endpoint.host_str().unwrap_or_default();
            if !matches!(endpoint.scheme(), "https" | "http")
                || (endpoint.scheme() != "https"
                    && !matches!(host, "localhost" | "127.0.0.1" | "[::1]"))
            {
                bail!(
                    "API-ul cloud trebuie să folosească HTTPS (HTTP este permis doar pe localhost)"
                );
            }
        }
        "mcp_servers_json" if !value.trim().is_empty() => {
            let registry: serde_json::Value =
                serde_json::from_str(value).context("Registry-ul MCP trebuie să fie JSON valid")?;
            if !registry.is_array() || registry.as_array().is_some_and(|items| items.len() > 100) {
                bail!("Registry-ul MCP trebuie să fie o listă JSON cu maximum 100 de servere");
            }
        }
        "custom_ai_providers" if !value.trim().is_empty() => {
            let providers: Vec<CustomAiProvider> = serde_json::from_str(value)
                .context("Providerii custom trebuie să fie o listă JSON validă")?;
            if providers.len() > 50 {
                bail!("Poți configura maximum 50 de provideri custom");
            }
            let mut ids = std::collections::HashSet::new();
            for provider in providers {
                custom_provider_credential_id(&provider.id)?;
                if !ids.insert(provider.id.clone()) {
                    bail!("ID-urile providerilor trebuie să fie unice");
                }
                if provider.name.trim().is_empty() || provider.name.len() > 80 {
                    bail!("Numele afișat trebuie să aibă 1–80 caractere");
                }
                if provider.model.trim().is_empty() || provider.model.len() > 160 {
                    bail!("Numele modelului trebuie să aibă 1–160 caractere");
                }
                if !matches!(
                    provider.protocol.as_str(),
                    "openai-compatible"
                        | "anthropic-messages"
                        | "google-gemini"
                        | "google-vertex-express"
                        | "azure-openai"
                ) {
                    bail!("Formatul API selectat nu este disponibil");
                }
                crate::llm::chat_completions_endpoint(&provider.base_url)?;
                if provider.api_key_env.len() > 128
                    || (!provider.api_key_env.is_empty()
                        && !provider.api_key_env.bytes().all(|byte| {
                            byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                        }))
                {
                    bail!("Numele variabilei de mediu poate conține doar majuscule, cifre și underscore");
                }
            }
        }
        "workflow_timeout_seconds" | "tool_command_timeout" if !value.trim().is_empty() => {
            let seconds = value
                .parse::<u64>()
                .context("Timeout-ul trebuie să fie un număr de secunde")?;
            if !(5..=3600).contains(&seconds) {
                bail!("Timeout-ul trebuie să fie între 5 și 3600 de secunde");
            }
        }
        "language" if !matches!(value, "ro-RO" | "en-US") => {
            bail!("Limba selectată nu este disponibilă");
        }
        "workflow_instructions" if value.len() > 16 * 1024 => {
            bail!("Instrucțiunile agentului nu pot depăși 16 KB");
        }
        "default_preview_url" if !value.trim().is_empty() => {
            validate_local_url(value.trim())?;
        }
        "external_database_host"
            if value.contains('@') || value.contains('/') || value.contains(':') =>
        {
            bail!("Introdu doar numele gazdei, fără utilizator, parolă sau URL");
        }
        _ => {}
    }
    Ok(())
}

#[tauri::command]
pub fn get_app_setting(key: String, state: State<'_, AppState>) -> CommandResult<Option<String>> {
    command((|| {
        if !APP_SETTING_KEYS.contains(&key.as_str()) {
            bail!("Setarea aplicației nu este permisă");
        }
        let value = state.db.get_setting(0, &key)?;
        if key == "update_endpoint" {
            Ok(value
                .filter(|endpoint| !endpoint.trim().is_empty())
                .or_else(|| DEFAULT_UPDATE_ENDPOINT.map(str::to_string)))
        } else {
            Ok(value)
        }
    })())
}

#[tauri::command]
pub fn set_app_setting(
    key: String,
    value: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    command((|| {
        validate_app_setting(&key, &value)?;
        state.db.set_setting(0, &key, &value)
    })())
}

#[tauri::command]
pub fn get_project_setting(
    project_id: i64,
    key: String,
    state: State<'_, AppState>,
) -> CommandResult<Option<String>> {
    command((|| {
        state.db.get_project(project_id)?;
        let value = state.db.get_setting(project_id, &key)?;
        if key == "preview_url" {
            value.map(|url| validate_local_url(&url)).transpose()
        } else {
            Ok(value)
        }
    })())
}

#[tauri::command]
pub fn set_project_setting(
    project_id: i64,
    key: String,
    mut value: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    command((|| {
        state.db.get_project(project_id)?;
        if !matches!(key.as_str(), "dev_command" | "preview_url") {
            bail!("Setarea proiectului nu este permisă");
        }
        if value.len() > 4_000 {
            bail!("Valoarea setării este prea lungă");
        }
        if key == "dev_command" {
            let parts = crate::process_manager::split_command_line(&value)?;
            if parts.is_empty() {
                bail!("Comanda dev-server este goală");
            }
            crate::process_manager::validate_command(&parts[0], &parts[1..])?;
        }
        if key == "preview_url" {
            value = validate_local_url(&value)?;
        }
        state.db.set_setting(project_id, &key, &value)
    })())
}

#[tauri::command]
pub fn validate_preview_url(value: String) -> CommandResult<String> {
    command(validate_local_url(&value))
}

#[tauri::command]
pub fn parse_command_line(value: String) -> CommandResult<Vec<String>> {
    command(crate::process_manager::split_command_line(&value))
}

#[tauri::command]
pub async fn start_dev_server(
    project_id: i64,
    program: String,
    args: Vec<String>,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<ProcessStatus> {
    let command_args = std::iter::once(program.clone())
        .chain(args.clone())
        .collect::<Vec<_>>();
    let action = format!(
        "process:start:{project_id}:{}:{}",
        program,
        serde_json::to_string(&command_args).map_err(|error| error.to_string())?
    );
    if let Err(error) = state.confirmations.consume(&confirmation_token, &action) {
        return Err(error.to_string());
    }
    let workspace = match workspace(&state.db, project_id) {
        Ok(value) => value,
        Err(error) => return Err(error.to_string()),
    };
    command(
        state
            .processes
            .start(project_id, workspace.root(), &program, &args)
            .await,
    )
}

#[tauri::command]
pub async fn stop_dev_server(project_id: i64, state: State<'_, AppState>) -> CommandResult<()> {
    command(state.processes.stop(project_id).await)
}

#[tauri::command]
pub async fn get_process_status(
    project_id: i64,
    state: State<'_, AppState>,
) -> CommandResult<ProcessStatus> {
    command(state.processes.status(project_id).await)
}

#[tauri::command]
pub fn create_snapshot(
    project_id: i64,
    message: String,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    command((|| {
        state.confirmations.consume(
            &confirmation_token,
            &format!("snapshot:create:{project_id}:{message}"),
        )?;
        git::create_snapshot(workspace(&state.db, project_id)?.root(), &message)
    })())
}

#[tauri::command]
pub fn list_snapshots(project_id: i64, state: State<'_, AppState>) -> CommandResult<Vec<Snapshot>> {
    command(workspace(&state.db, project_id).and_then(|value| git::list_snapshots(value.root())))
}

#[tauri::command]
pub fn rollback_snapshot(
    project_id: i64,
    hash: String,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    command((|| {
        state.confirmations.consume(
            &confirmation_token,
            &format!("snapshot:rollback:{project_id}:{hash}"),
        )?;
        git::rollback(workspace(&state.db, project_id)?.root(), &hash)
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_structured_change() {
        let text = "Gata.\n<nomad-change>{\"path\":\"src/a.ts\",\"summary\":\"Test\",\"content\":\"hello\"}</nomad-change>";
        let change = extract_change(text).unwrap().unwrap();
        assert_eq!(change.path, "src/a.ts");
        assert_eq!(change.content, "hello");
    }

    #[test]
    fn starter_defaults_use_direct_node_and_loopback_ip_without_overwriting_user_settings() {
        let directory = tempfile::tempdir().unwrap();
        let db = Database::open(&directory.path().join("nomad.sqlite3")).unwrap();
        let project = db
            .upsert_project("starter", directory.path().to_str().unwrap())
            .unwrap();
        ensure_starter_settings(&db, project.id).unwrap();
        assert_eq!(
            db.get_setting(project.id, "dev_command")
                .unwrap()
                .as_deref(),
            Some("node dev-server.mjs")
        );
        assert_eq!(
            db.get_setting(project.id, "preview_url")
                .unwrap()
                .as_deref(),
            Some("http://127.0.0.1:5173")
        );

        db.set_setting(project.id, "dev_command", "npm run custom")
            .unwrap();
        ensure_starter_settings(&db, project.id).unwrap();
        assert_eq!(
            db.get_setting(project.id, "dev_command")
                .unwrap()
                .as_deref(),
            Some("npm run custom")
        );
    }

    #[cfg(windows)]
    #[test]
    fn removes_windows_verbatim_prefix_for_display() {
        assert_eq!(display_path(Path::new(r"\\?\C:\work\app")), r"C:\work\app");
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share")),
            r"\\server\share"
        );
    }

    #[test]
    fn detects_nested_repository_parent() {
        let directory = tempfile::tempdir().unwrap();
        let repository = directory.path().join("repo");
        let nested = repository.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(repository.join(".git")).unwrap();
        assert!(is_inside_repository(&nested).unwrap());
        assert!(!is_inside_repository(directory.path()).unwrap());
    }

    #[test]
    fn project_export_includes_source_and_omits_secrets_and_build_folders() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let target = directory.path().join("export");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(source.join("node_modules")).unwrap();
        fs::create_dir_all(source.join(".git")).unwrap();
        fs::write(source.join("src/main.ts"), "source").unwrap();
        fs::write(source.join(".env"), "secret").unwrap();
        fs::write(source.join("node_modules/package.json"), "generated").unwrap();
        fs::write(source.join(".git/config"), "git metadata").unwrap();
        fs::create_dir(&target).unwrap();

        let copied = copy_export_tree(&source, &target).unwrap();

        assert_eq!(copied, 1);
        assert_eq!(
            fs::read_to_string(target.join("src/main.ts")).unwrap(),
            "source"
        );
        assert!(!target.join(".env").exists());
        assert!(!target.join("node_modules").exists());
        assert!(!target.join(".git").exists());
    }
}
