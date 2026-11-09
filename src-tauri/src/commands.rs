use crate::agent_tools;
use crate::confirmations::ConfirmationManager;
use crate::db::Database;
use crate::git;
use crate::llm::{LlmMessage, LlmProvider, LlmRequest, OpenAiProvider};
use crate::models::*;
use crate::process_manager::ProcessManager;
use crate::security::Workspace;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

pub struct AppState {
    pub db: Arc<Database>,
    pub confirmations: Arc<ConfirmationManager>,
    pub processes: Arc<ProcessManager>,
}

type CommandResult<T> = std::result::Result<T, String>;

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
        let root = parent.join(&name);
        if root.exists() {
            bail!("Directorul proiectului există deja");
        }
        fs::create_dir(&root)?;
        starter_files(&root, &name)?;
        let canonical = fs::canonicalize(&root)?;
        state.db.upsert_project(&name, &display_path(&canonical))
    })())
}

#[tauri::command]
pub fn open_project(path: String, state: State<'_, AppState>) -> CommandResult<Project> {
    command((|| {
        let root = fs::canonicalize(&path).context("Directorul nu există")?;
        if !root.is_dir() {
            bail!("Calea nu este un director");
        }
        let name = root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Proiect");
        state.db.upsert_project(name, &display_path(&root))
    })())
}

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> CommandResult<Vec<Project>> {
    command(state.db.list_projects())
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

#[tauri::command]
pub async fn run_command(
    project_id: i64,
    program: String,
    args: Vec<String>,
    timeout_seconds: u64,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<CommandOutput> {
    let action = format!("command:run:{project_id}:{program}:{}", args.join(" "));
    if let Err(error) = state.confirmations.consume(&confirmation_token, &action) {
        return Err(error.to_string());
    }
    let workspace = match workspace(&state.db, project_id) {
        Ok(value) => value,
        Err(error) => return Err(error.to_string()),
    };
    command(agent_tools::run_command(&workspace, &program, &args, timeout_seconds).await)
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
    command(state.db.list_messages(project_id, 100))
}

#[tauri::command]
pub fn list_proposals(project_id: i64, state: State<'_, AppState>) -> CommandResult<Vec<Proposal>> {
    command(state.db.list_proposals(project_id))
}

#[tauri::command]
pub fn create_manual_proposal(
    project_id: i64,
    path: String,
    content: String,
    summary: String,
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
        state
            .db
            .create_proposal(project_id, &path, &original, &content, &summary)
    })())
}

#[tauri::command]
pub fn reject_proposal(proposal_id: i64, state: State<'_, AppState>) -> CommandResult<Proposal> {
    command(state.db.update_proposal_status(proposal_id, "rejected"))
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
    prompt: String,
    active_path: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    if prompt.trim().is_empty() {
        return Err("Mesajul este gol".into());
    }
    let provider = OpenAiProvider::from_keychain().map_err(|error| error.to_string())?;
    let workspace = workspace(&state.db, project_id).map_err(|error| error.to_string())?;
    state
        .db
        .insert_message(project_id, "user", prompt.trim())
        .map_err(|error| error.to_string())?;
    let history = state
        .db
        .list_messages(project_id, 16)
        .map_err(|error| error.to_string())?;
    let files = agent_tools::list_files(&workspace).map_err(|error| error.to_string())?;
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
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let model = state
        .db
        .get_setting(0, "openai_model")
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "gpt-6-astra".into());
    let instructions = format!(
        r#"Ești agentul de programare din Nomad, un mediu local-first. Răspunde concis în limba utilizatorului.
În acest MVP poți analiza contextul și propune cel mult o schimbare de fișier. Nu pretinde că ai executat unelte sau comenzi.
Când este necesară o modificare, încheie răspunsul cu exact un bloc pe o singură linie:
<nomad-change>{{"path":"cale/relativă","summary":"rezumat scurt","content":"conținutul COMPLET al fișierului"}}</nomad-change>
Calea trebuie să fie relativă la proiect. Nu folosi .., căi absolute sau symlink-uri. Nu include blocul dacă utilizatorul cere doar explicații.
Uneltele backend disponibile după confirmarea utilizatorului sunt: {}.

Fișiere proiect:
{}{}
"#,
        agent_tools::TOOL_NAMES.join(", "),
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
    };
    let request_id = Uuid::new_v4().to_string();
    let returned_id = request_id.clone();
    let db = state.db.clone();
    let app_handle = app.clone();
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
        let response = provider.stream(request, delta_tx).await;
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
                    let message = db.insert_message(project_id, "assistant", &content)?;
                    Ok((message, proposal))
                })();
                match result {
                    Ok((message, proposal)) => {
                        let _ = app_handle.emit(
                            "chat://done",
                            StreamDone {
                                request_id,
                                message,
                                proposal,
                            },
                        );
                    }
                    Err(error) => {
                        let _ = app_handle.emit(
                            "chat://error",
                            StreamError {
                                request_id,
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
                        request_id,
                        message: error.to_string(),
                    },
                );
            }
        }
    });
    Ok(returned_id)
}

#[tauri::command]
pub fn save_api_key(api_key: String) -> CommandResult<()> {
    command(OpenAiProvider::save_key(&api_key))
}

#[tauri::command]
pub fn has_api_key() -> bool {
    OpenAiProvider::has_key()
}

#[tauri::command]
pub fn get_app_setting(key: String, state: State<'_, AppState>) -> CommandResult<Option<String>> {
    command(state.db.get_setting(0, &key))
}

#[tauri::command]
pub fn set_app_setting(
    key: String,
    value: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    command(state.db.set_setting(0, &key, &value))
}

#[tauri::command]
pub fn get_project_setting(
    project_id: i64,
    key: String,
    state: State<'_, AppState>,
) -> CommandResult<Option<String>> {
    command(state.db.get_setting(project_id, &key))
}

#[tauri::command]
pub fn set_project_setting(
    project_id: i64,
    key: String,
    value: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    command(state.db.set_setting(project_id, &key, &value))
}

#[tauri::command]
pub async fn start_dev_server(
    project_id: i64,
    program: String,
    args: Vec<String>,
    confirmation_token: String,
    state: State<'_, AppState>,
) -> CommandResult<ProcessStatus> {
    let action = format!(
        "process:start:{project_id}:{program}{}",
        if args.is_empty() {
            String::new()
        } else {
            format!(" {}", args.join(" "))
        }
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

    #[cfg(windows)]
    #[test]
    fn removes_windows_verbatim_prefix_for_display() {
        assert_eq!(display_path(Path::new(r"\\?\C:\work\app")), r"C:\work\app");
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share")),
            r"\\server\share"
        );
    }
}
