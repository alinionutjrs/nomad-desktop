import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Bot, Box, Check, ChevronRight, CircleStop, Clock3, Code2, File,
  FileCode2, Folder, FolderDown, FolderOpen, GitCommitHorizontal, History, KeyRound,
  LoaderCircle, MessageSquareText, Play, Plus, RefreshCw, RotateCcw,
  Save, Search, Settings, Sparkles, TerminalSquare, X,
} from "lucide-react";
import { createLineDiff } from "./lib/diff";
import { processStartConfirmationAction, splitCommandLine } from "./lib/command";
import { confirmedInvoke } from "./lib/tauri";
import type {
  ChatMessage, ChatSession, FileEntry, ProcessStatus, Project, Proposal, Snapshot,
  StreamDelta, StreamDone, StreamError,
} from "./types";

type View = "agent" | "editor" | "preview" | "history" | "changes" | "settings";
type SettingsSection = "general" | "updates" | "profile" | "ai" | "workflow" | "tools" | "integrations" | "permissions" | "advanced" | "experiments" | "help" | "danger";
type Theme = "forest" | "ocean" | "violet";
type Density = "comfortable" | "compact";
type AiCliStatus = { id: string; available: boolean; executable: string | null; help_url: string };
type ApiProtocol = "openai-compatible" | "anthropic-messages" | "google-gemini" | "google-vertex-express" | "azure-openai";
type CustomAiProvider = { id: string; name: string; baseUrl: string; model: string; protocol: ApiProtocol; apiKeyEnv: string };
const aiProviderTemplates: { id: string; label: string; provider: CustomAiProvider; modelHint: string; endpointHint?: string }[] = [
  { id: "openai", label: "OpenAI", provider: { id: "openai", name: "OpenAI", baseUrl: "https://api.openai.com/v1", model: "", protocol: "openai-compatible", apiKeyEnv: "OPENAI_API_KEY" }, modelHint: "ID-ul modelului OpenAI" },
  { id: "anthropic", label: "Anthropic", provider: { id: "anthropic", name: "Anthropic", baseUrl: "https://api.anthropic.com", model: "", protocol: "anthropic-messages", apiKeyEnv: "ANTHROPIC_API_KEY" }, modelHint: "ex: claude-sonnet-4-6" },
  { id: "google", label: "Google Gemini", provider: { id: "google-gemini", name: "Google Gemini", baseUrl: "https://generativelanguage.googleapis.com/v1beta", model: "", protocol: "google-gemini", apiKeyEnv: "GEMINI_API_KEY" }, modelHint: "ex: gemini-2.5-flash" },
  { id: "vertex", label: "Google Vertex AI · Express", provider: { id: "google-vertex", name: "Google Vertex AI", baseUrl: "https://aiplatform.googleapis.com/v1", model: "", protocol: "google-vertex-express", apiKeyEnv: "GOOGLE_VERTEX_API_KEY" }, modelHint: "ex: gemini-2.5-flash" },
  { id: "openrouter", label: "OpenRouter", provider: { id: "openrouter", name: "OpenRouter", baseUrl: "https://openrouter.ai/api/v1", model: "", protocol: "openai-compatible", apiKeyEnv: "OPENROUTER_API_KEY" }, modelHint: "ex: provider/model · free models are available" },
  { id: "azure", label: "Azure OpenAI", provider: { id: "azure-openai", name: "Azure OpenAI", baseUrl: "", model: "", protocol: "azure-openai", apiKeyEnv: "AZURE_OPENAI_API_KEY" }, modelHint: "numele deployment-ului Azure", endpointHint: "https://numele-resursei.openai.azure.com/openai/v1" },
  { id: "bedrock", label: "Amazon Bedrock", provider: { id: "amazon-bedrock", name: "Amazon Bedrock", baseUrl: "", model: "", protocol: "openai-compatible", apiKeyEnv: "AWS_BEARER_TOKEN_BEDROCK" }, modelHint: "ID-ul modelului Bedrock disponibil în regiunea ta", endpointHint: "https://bedrock-mantle.REGION.api.aws/v1" },
  { id: "xai", label: "xAI", provider: { id: "xai", name: "xAI", baseUrl: "https://api.x.ai/v1", model: "", protocol: "openai-compatible", apiKeyEnv: "XAI_API_KEY" }, modelHint: "ID-ul modelului xAI" },
  { id: "minimax", label: "MiniMax", provider: { id: "minimax", name: "MiniMax", baseUrl: "https://api.minimax.io/v1", model: "", protocol: "openai-compatible", apiKeyEnv: "MINIMAX_API_KEY" }, modelHint: "ex: MiniMax-M3.1-Flash" },
  { id: "ollama", label: "Ollama local", provider: { id: "ollama", name: "Ollama local", baseUrl: "http://localhost:11434/v1", model: "", protocol: "openai-compatible", apiKeyEnv: "" }, modelHint: "numele modelului instalat local" },
  { id: "custom", label: "Alt provider OpenAI-compatible", provider: { id: "", name: "", baseUrl: "", model: "", protocol: "openai-compatible", apiKeyEnv: "" }, modelHint: "numele modelului providerului" },
];
type AppUpdateInfo = { version: string; currentVersion: string; notes: string | null };

const emptyProcess: ProcessStatus = { running: false, pid: null, started_at: null, logs: [] };

function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [project, setProject] = useState<Project | null>(null);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState("");
  const [draft, setDraft] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [sessions, setSessions] = useState<ChatSession[]>([]);
  const [sessionId, setSessionId] = useState<number | null>(null);
  const [activeSessionId, setActiveSessionId] = useState<number | null>(null);
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [streaming, setStreaming] = useState("");
  const [activeRequest, setActiveRequest] = useState<string | null>(null);
  const [prompt, setPrompt] = useState("");
  const [view, setView] = useState<View>("agent");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [processStatus, setProcessStatus] = useState<ProcessStatus>(emptyProcess);
  const [devCommand, setDevCommand] = useState("npm run dev -- --host 127.0.0.1");
  const [previewUrl, setPreviewUrl] = useState("http://127.0.0.1:5173");
  const [previewKey, setPreviewKey] = useState(0);
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [workingDiff, setWorkingDiff] = useState("");
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("ai");
  const [theme, setTheme] = useState<Theme>("forest");
  const [density, setDensity] = useState<Density>("comfortable");
  const [availableUpdate, setAvailableUpdate] = useState<AppUpdateInfo | null>(null);
  const [updatingApp, setUpdatingApp] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const activeRequestRef = useRef<string | null>(null);

  const showError = useCallback((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    setNotice(message);
    window.setTimeout(() => setNotice(null), 6000);
  }, []);

  const refreshProjects = useCallback(async () => {
    const value = await invoke<Project[]>("list_projects");
    setProjects(value);
    return value;
  }, []);

  const loadProject = useCallback(async (next: Project) => {
    setProject(next);
    setProcessStatus(emptyProcess);
    setSelectedPath(null);
    setFileContent("");
    setDraft("");
    const [nextFiles, nextProposals, command, url, defaultCommand, defaultUrl] = await Promise.all([
      invoke<FileEntry[]>("list_project_files", { projectId: next.id }),
      invoke<Proposal[]>("list_proposals", { projectId: next.id }),
      invoke<string | null>("get_project_setting", { projectId: next.id, key: "dev_command" }),
      invoke<string | null>("get_project_setting", { projectId: next.id, key: "preview_url" }),
      invoke<string | null>("get_app_setting", { key: "default_dev_command" }),
      invoke<string | null>("get_app_setting", { key: "default_preview_url" }),
    ]);
    const nextSessions = await invoke<ChatSession[]>("list_chat_sessions", { projectId: next.id });
    setSessions(nextSessions);
    setSessionId(nextSessions[0]?.id ?? null);
    setActiveSessionId(nextSessions[0]?.id ?? null);
    const nextMessages = nextSessions[0]
      ? await invoke<ChatMessage[]>("list_session_messages", { sessionId: nextSessions[0].id })
      : [];
    setFiles(nextFiles);
    setMessages(nextMessages);
    setProposals(nextProposals);
    setDevCommand(command ?? defaultCommand ?? "npm run dev -- --host 127.0.0.1");
    setPreviewUrl(url ?? defaultUrl ?? "http://127.0.0.1:5173");
  }, []);

  const loadWorkingDiff = useCallback(async () => {
    if (!project) return;
    try { setWorkingDiff(await invoke<string>("git_diff", { projectId: project.id })); }
    catch (error) { showError(error); }
  }, [project, showError]);

  const refreshProject = useCallback(async () => {
    if (!project) return;
    const [nextFiles, nextProposals] = await Promise.all([
      invoke<FileEntry[]>("list_project_files", { projectId: project.id }),
      invoke<Proposal[]>("list_proposals", { projectId: project.id }),
    ]);
    setFiles(nextFiles);
    setProposals(nextProposals);
  }, [project]);

  useEffect(() => {
    refreshProjects().catch(showError);
  }, [refreshProjects, showError]);

  useEffect(() => {
    invoke<string | null>("get_app_setting", { key: "theme" }).then((value) => {
      if (value === "ocean" || value === "violet" || value === "forest") setTheme(value);
    }).catch(showError);
    invoke<string | null>("get_app_setting", { key: "ui_density" }).then((value) => {
      if (value === "compact" || value === "comfortable") setDensity(value);
    }).catch(() => {});
  }, [showError]);

  const checkForAppUpdate = useCallback(async () => {
    const update = await invoke<AppUpdateInfo | null>("check_for_update");
    setAvailableUpdate(update);
    return update;
  }, []);

  useEffect(() => {
    let active = true;
    invoke<string | null>("get_app_setting", { key: "update_endpoint" })
      .then((endpoint) => endpoint?.trim() ? invoke<AppUpdateInfo | null>("check_for_update") : null)
      .then((update) => { if (active) setAvailableUpdate(update); })
      .catch(() => { /* Update checks are best effort at startup. */ });
    return () => { active = false; };
  }, []);

  const installAppUpdate = async () => {
    setUpdatingApp(true);
    try { await invoke("install_app_update"); }
    catch (error) { showError(error); setUpdatingApp(false); }
  };

  const changeTheme = async (value: Theme) => {
    setTheme(value);
    try { await invoke("set_app_setting", { key: "theme", value }); }
    catch (error) { showError(error); }
  };

  const changeDensity = async (value: Density) => {
    setDensity(value);
    try { await invoke("set_app_setting", { key: "ui_density", value }); }
    catch (error) { showError(error); }
  };

  useEffect(() => {
    const unlisteners = [
      listen<StreamDelta>("chat://delta", ({ payload }) => {
        if (payload.request_id === activeRequestRef.current && activeSessionId === sessionId) {
          setStreaming((value) => value + payload.delta);
        }
      }),
      listen<StreamDone>("chat://done", ({ payload }) => {
        if (payload.request_id !== activeRequestRef.current) return;
        if (activeSessionId === sessionId) setMessages((value) => [...value, payload.message]);
        if (payload.proposal) setProposals((value) => [payload.proposal!, ...value]);
        setStreaming("");
        setActiveRequest(null);
        activeRequestRef.current = null;
      }),
      listen<StreamError>("chat://error", ({ payload }) => {
        if (payload.request_id !== activeRequestRef.current) return;
        showError(payload.message);
        setStreaming("");
        setActiveRequest(null);
        activeRequestRef.current = null;
      }),
      listen<{ request_id: string }>("chat://cancelled", ({ payload }) => {
        if (payload.request_id !== activeRequestRef.current) return;
        setStreaming("");
        setActiveRequest(null);
        activeRequestRef.current = null;
      }),
    ];
    return () => { void Promise.all(unlisteners).then((items) => items.forEach((fn) => fn())); };
  }, [showError, activeSessionId]);

  useEffect(() => {
    if (!project) return;
    const poll = async () => {
      try {
        setProcessStatus(await invoke<ProcessStatus>("get_process_status", { projectId: project.id }));
      } catch (error) {
        showError(error);
      }
    };
    void poll();
    const timer = window.setInterval(poll, 1500);
    return () => window.clearInterval(timer);
  }, [project, showError]);

  const chooseCreate = async () => {
    const parent = await open({ directory: true, multiple: false, title: "Alege directorul părinte" });
    if (typeof parent !== "string") return;
    const name = window.prompt("Numele proiectului", "nomad-app")?.trim();
    if (!name) return;
    try {
      const created = await confirmedInvoke<Project>(
        "create_project", { parentDir: parent, name }, `project:create:${parent}:${name}`,
        `Creezi proiectul „${name}” în ${parent}?`,
      );
      if (!created) return;
      await refreshProjects();
      await loadProject(created);
    } catch (error) { showError(error); }
  };

  const chooseOpen = async () => {
    const path = await open({ directory: true, multiple: false, title: "Deschide un proiect" });
    if (typeof path !== "string") return;
    try {
      const opened = await invoke<Project>("open_project", { path });
      await refreshProjects();
      await loadProject(opened);
    } catch (error) { showError(error); }
  };

  const exportProject = async () => {
    if (!project) return;
    const parent = await open({ directory: true, multiple: false, title: "Alege directorul pentru export" });
    if (typeof parent !== "string") return;
    try {
      const result = await confirmedInvoke<string>(
        "export_project_files",
        { projectId: project.id, destinationParent: parent },
        `project:export:${project.id}:${parent}`,
        `Exporți o copie a proiectului „${project.name}” în ${parent}? Fișierele cu credențiale și folderele de build vor fi omise.`,
      );
      if (!result) return;
      const [path, count] = result.split("|");
      setNotice(`Exportat ${count ?? "0"} fișiere în ${path}.`);
      window.setTimeout(() => setNotice(null), 6000);
    } catch (error) { showError(error); }
  };

  const selectFile = async (entry: FileEntry) => {
    if (!project || entry.is_dir) return;
    try {
      const content = await invoke<string>("read_file", { projectId: project.id, path: entry.path });
      setSelectedPath(entry.path);
      setFileContent(content);
      setDraft(content);
      setView("editor");
    } catch (error) { showError(error); }
  };

  const sendPrompt = async () => {
    const value = prompt.trim();
    if (!project || !value || activeRequest) return;
    const optimistic: ChatMessage = {
      id: -Date.now(), role: "user", content: value, created_at: new Date().toISOString(),
    };
    setPrompt("");
    setStreaming("");
    try {
      let targetSession = sessionId;
      if (!targetSession) {
        targetSession = await invoke<number>("create_chat_session", { projectId: project.id, title: value });
        setSessionId(targetSession);
        setActiveSessionId(targetSession);
        setSessions(await invoke<ChatSession[]>("list_chat_sessions", { projectId: project.id }));
      }
      const requestId = await invoke<string>("chat_stream", {
        projectId: project.id, sessionId: targetSession, prompt: value, activePath: selectedPath,
      });
      activeRequestRef.current = requestId;
      setActiveRequest(requestId);
      if (activeSessionId === targetSession) setMessages((items) => [...items, optimistic]);
    } catch (error) { showError(error); }
  };

  const cancelChat = async () => {
    if (!activeRequest) return;
    try { await invoke("cancel_chat", { requestId: activeRequest }); }
    catch (error) { showError(error); }
  };

  const acceptProposal = async (proposal: Proposal) => {
    if (!project) return;
    setBusy(true);
    try {
      const accepted = await confirmedInvoke<Proposal>(
        "accept_proposal", { projectId: project.id, proposalId: proposal.id },
        `proposal:accept:${proposal.id}`,
        `Aplici modificarea propusă în ${proposal.path}? Înainte de scriere se creează un snapshot Git.`,
      );
      if (!accepted) return;
      await refreshProject();
      if (selectedPath === proposal.path) await selectFile({ path: proposal.path, is_dir: false, size: 0, depth: 0 });
    } catch (error) { showError(error); }
    finally { setBusy(false); }
  };

  const rejectProposal = async (proposal: Proposal) => {
    try {
      await invoke("reject_proposal", { proposalId: proposal.id });
      await refreshProject();
    } catch (error) { showError(error); }
  };

  const proposeEditorChange = async () => {
    if (!project || !selectedPath || draft === fileContent) return;
    try {
      const proposal = await invoke<Proposal>("create_manual_proposal", {
        projectId: project.id, path: selectedPath, content: draft,
        summary: `Modificare manuală pentru ${selectedPath}`, expectedOriginal: fileContent,
      });
      setProposals((items) => [proposal, ...items]);
      setView("agent");
    } catch (error) { showError(error); }
  };

  const startServer = async () => {
    if (!project) return;
    try {
      const parts = await invoke<string[]>("parse_command_line", { value: devCommand });
      if (!parts.length) throw new Error("Introdu o comandă dev-server.");
      const safeUrl = await invoke<string>("validate_preview_url", { value: previewUrl });
      setPreviewUrl(safeUrl);
      await Promise.all([
        invoke("set_project_setting", { projectId: project.id, key: "dev_command", value: devCommand }),
        invoke("set_project_setting", { projectId: project.id, key: "preview_url", value: safeUrl }),
      ]);
      await confirmedInvoke(
        "start_dev_server", { projectId: project.id, program: parts[0], args: parts.slice(1) },
        processStartConfirmationAction(project.id, parts),
        `Pornești în proiect comanda „${devCommand}”? Aceasta va rula până o oprești.`,
      );
      setView("preview");
    } catch (error) { showError(error); }
  };

  const stopServer = async () => {
    if (!project) return;
    try { await invoke("stop_dev_server", { projectId: project.id }); }
    catch (error) { showError(error); }
  };

  const createSnapshot = async () => {
    if (!project) return;
    const label = window.prompt("Etichetă snapshot", "Snapshot manual")?.trim();
    if (!label) return;
    try {
      await confirmedInvoke(
        "create_snapshot", { projectId: project.id, message: label },
        `snapshot:create:${project.id}:${label}`, `Creezi snapshot-ul Git „${label}”?`,
      );
      await loadSnapshots();
    } catch (error) { showError(error); }
  };

  const loadSnapshots = async () => {
    if (!project) return;
    try { setSnapshots(await invoke<Snapshot[]>("list_snapshots", { projectId: project.id })); }
    catch (error) { showError(error); }
  };

  const rollback = async (snapshot: Snapshot) => {
    if (!project) return;
    try {
      const value = await confirmedInvoke(
        "rollback_snapshot", { projectId: project.id, hash: snapshot.hash },
        `snapshot:rollback:${project.id}:${snapshot.hash}`,
        `Restaurezi fișierele proiectului la snapshot-ul „${snapshot.message}” (${snapshot.hash.slice(0, 8)})? Starea curentă va primi mai întâi un snapshot de siguranță.`,
      );
      if (value !== null) await refreshProject();
    } catch (error) { showError(error); }
  };

  useEffect(() => {
    if (view === "history") void loadSnapshots();
  }, [view, project]); // eslint-disable-line react-hooks/exhaustive-deps

  const pending = useMemo(() => proposals.filter((item) => item.status === "pending"), [proposals]);

  return (
    <div className={`app-shell ${sidebarCollapsed ? "sidebar-collapsed" : ""}`} data-theme={theme} data-density={density}>
      <aside className="sidebar">
        <div className="brand"><button className="brand-mark" title="Restrânge meniul" onClick={() => setSidebarCollapsed((value) => !value)}><img src="/nomad-mark.svg" alt="" /></button><span>nomad</span><em>local</em></div>
        <div className="sidebar-heading"><span>Proiecte</span><div>
          <button className="icon-button" aria-label="Deschide proiect" title="Deschide proiect" onClick={chooseOpen}><FolderOpen size={17} /></button>
          {project && <button className="icon-button" aria-label="Exportă proiectul" title="Exportă o copie a proiectului" onClick={() => void exportProject()}><FolderDown size={17} /></button>}
          <button className="icon-button" aria-label="Proiect nou" title="Proiect nou" onClick={chooseCreate}><Plus size={17} /></button>
        </div></div>
        <div className="project-list">
          {projects.map((item) => (
            <button key={item.id} className={`project-item ${project?.id === item.id ? "active" : ""}`} onClick={() => loadProject(item).catch(showError)}>
              <Box size={15} /><span><strong>{item.name}</strong><small>{item.path}</small></span>
            </button>
          ))}
          {!projects.length && <div className="empty-mini">Deschide sau creează primul proiect.</div>}
        </div>
        <div className="sidebar-heading files-heading"><span>Fișiere</span><button className="icon-button" aria-label="Reîncarcă fișierele" title="Reîncarcă fișierele" onClick={() => refreshProject().catch(showError)}><RefreshCw size={16} /></button></div>
        <div className="file-tree">
          {files.map((entry) => (
            <button key={entry.path} className={`file-row ${selectedPath === entry.path ? "active" : ""}`} style={{ paddingLeft: 10 + entry.depth * 13 }} onClick={() => selectFile(entry)} disabled={entry.is_dir}>
              {entry.is_dir ? <Folder size={14} /> : <FileCode2 size={14} />}<span>{entry.path.split("/").at(-1)}</span>
            </button>
          ))}
        </div>
        <button className="settings-link" onClick={() => { setSettingsSection("general"); setView("settings"); }}><Settings size={15} /> Setări</button>
      </aside>

      <main className="workspace">
        <header className="topbar">
          <div className="project-crumb"><span>{project?.name ?? "Niciun proiect"}</span>{selectedPath && <><ChevronRight size={13} /><small>{selectedPath}</small></>}</div>
          <nav>
            <Tab icon={<MessageSquareText size={15} />} label="Agent" active={view === "agent"} badge={pending.length} onClick={() => setView("agent")} />
            <Tab icon={<Code2 size={15} />} label="Editor" active={view === "editor"} onClick={() => setView("editor")} />
            <Tab icon={<Play size={14} />} label="Preview" active={view === "preview"} dot={processStatus.running} onClick={() => setView("preview")} />
            <Tab icon={<History size={15} />} label="Istoric" active={view === "history"} onClick={() => setView("history")} />
            <Tab icon={<GitCommitHorizontal size={15} />} label="Diff" active={view === "changes"} onClick={() => setView("changes")} />
          </nav>
        </header>

        {availableUpdate && <div className="update-banner"><div><strong>Nomad {availableUpdate.version} este disponibil.</strong><span>{availableUpdate.notes || "Actualizarea este semnată și va fi instalată în siguranță."}</span></div><button className="primary-small" disabled={updatingApp} onClick={installAppUpdate}>{updatingApp ? <LoaderCircle className="spin" size={14} /> : <RefreshCw size={14} />}{updatingApp ? "Se instalează…" : "Actualizează și repornește"}</button><button className="icon-button" title="Mai târziu" onClick={() => setAvailableUpdate(null)}><X size={14} /></button></div>}

        {project && view === "agent" && <div className="session-strip"><span>Sesiunea curentă</span><select value={sessionId ?? ""} onChange={async (event) => {
          const id = Number(event.target.value);
          setSessionId(id);
          setActiveSessionId(id);
          try { setMessages(await invoke<ChatMessage[]>("list_session_messages", { sessionId: id })); }
          catch (error) { showError(error); }
        }}><option value="" disabled>Alege o sesiune</option>{sessions.map((session) => <option key={session.id} value={session.id}>{session.title}</option>)}</select><button onClick={async () => {
          try {
            const id = await invoke<number>("create_chat_session", { projectId: project.id, title: "Conversație nouă" });
            const updated = await invoke<ChatSession[]>("list_chat_sessions", { projectId: project.id });
            setSessions(updated); setSessionId(id); setMessages([]);
            setActiveSessionId(id);
          } catch (error) { showError(error); }
        }}><Plus size={13} /> Conversație nouă</button></div>}

        {!project && view !== "settings" && <Welcome onCreate={chooseCreate} onOpen={chooseOpen} />}
        {project && view === "agent" && (
          <section className="agent-view">
            <div className="conversation">
              {!messages.length && !streaming && <div className="agent-empty"><Bot size={28} /><h2>Ce construim?</h2><p>Descrie schimbarea. Nomad o va propune mai întâi, apoi tu decizi dacă ajunge pe disc.</p></div>}
              {messages.map((message) => <MessageBubble key={message.id} message={message} />)}
              {streaming && <div className="message assistant"><div className="avatar"><Bot size={14} /></div><div><span className="streaming-dot" />{streaming}</div></div>}
            </div>
            {pending.length > 0 && <ProposalCard proposal={pending[0]} busy={busy} onAccept={acceptProposal} onReject={rejectProposal} />}
            <div className="composer">
              <textarea value={prompt} onChange={(event) => setPrompt(event.target.value)} placeholder="Cere o funcție, o corecție sau o explicație…" onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void sendPrompt(); } }} />
              <div className="composer-footer"><span>{selectedPath ? `Context: ${selectedPath}` : "Context: proiect"}</span><button onClick={activeRequest ? cancelChat : sendPrompt} disabled={!activeRequest && !prompt.trim()}>{activeRequest ? <><X size={15} /> Oprește</> : <><Sparkles size={16} /> Trimite</>}</button></div>
            </div>
          </section>
        )}

        {project && view === "editor" && (
          <section className="editor-view">
            {selectedPath ? <>
              <div className="panel-toolbar"><div><File size={15} /> {selectedPath}</div><button className="icon-button" aria-label="Reîncarcă fișierul" title="Reîncarcă fișierul de pe disc" onClick={() => selectFile({ path: selectedPath, is_dir: false, size: 0, depth: 0 })}><RefreshCw size={16} /></button><button className="primary-small" disabled={draft === fileContent} onClick={proposeEditorChange}><Save size={16} /> Propune salvarea</button></div>
              <textarea className="code-editor" spellCheck={false} value={draft} onChange={(event) => setDraft(event.target.value)} />
              {draft !== fileContent && <div className="editor-status">Modificările sunt locale în editor. „Propune salvarea” creează un diff aprobabil.</div>}
            </> : <EmptyPanel icon={<Code2 size={25} />} title="Selectează un fișier" body="Fișierul se deschide aici fără a fi modificat." />}
          </section>
        )}

        {project && view === "preview" && (
          <section className="preview-view">
            <div className="preview-toolbar">
              <span className={`status-pill ${processStatus.running ? "online" : ""}`}>{processStatus.running ? "Rulează" : "Oprit"}</span>
              <input value={previewUrl} onChange={(event) => setPreviewUrl(event.target.value)} />
              <button className="icon-button" aria-label="Reîncarcă preview" title="Reîncarcă preview" onClick={() => setPreviewKey((value) => value + 1)}><RefreshCw size={17} /></button>
              {processStatus.running ? <button className="danger-small" onClick={stopServer}><CircleStop size={14} /> Oprește</button> : <button className="primary-small" onClick={startServer}><Play size={14} /> Pornește</button>}
            </div>
            <div className="command-row"><TerminalSquare size={14} /><input value={devCommand} onChange={(event) => setDevCommand(event.target.value)} /></div>
            <div className="preview-content">
              {processStatus.running
                ? <iframe key={`${project.id}-${previewKey}`} title="Preview aplicație" src={previewUrl} sandbox="allow-scripts allow-forms allow-modals allow-popups" />
                : <div className="preview-placeholder"><Play size={26} /><strong>Preview oprit</strong><span>Pornește serverul proiectului pentru a vedea aplicația.</span></div>}
              <div className="logs"><div className="logs-title">Loguri proces {processStatus.pid && `· PID ${processStatus.pid}`}</div><pre>{processStatus.logs.slice(-80).join("\n") || "Pornește dev-server-ul pentru a vedea logurile."}</pre></div>
            </div>
          </section>
        )}

        {project && view === "changes" && (
          <section className="history-view">
            <div className="history-header"><div><h2>Schimbări locale</h2><p>Diferența față de ultimul snapshot.</p></div><button className="primary-small" onClick={loadWorkingDiff}><RefreshCw size={14} /> Actualizează</button></div>
            <pre className="working-diff">{workingDiff || "Nu există schimbări față de ultimul snapshot."}</pre>
          </section>
        )}

        {project && view === "history" && (
          <section className="history-view">
            <div className="history-header"><div><h2>Snapshot-uri Git</h2><p>Puncte locale de revenire, fără remote.</p></div><button className="primary-small" onClick={createSnapshot}><GitCommitHorizontal size={14} /> Snapshot nou</button></div>
            <div className="snapshot-list">{snapshots.map((snapshot) => <div className="snapshot" key={snapshot.hash}><div className="snapshot-dot" /><div><strong>{snapshot.message}</strong><span><code>{snapshot.hash.slice(0, 9)}</code> · {new Date(snapshot.created_at).toLocaleString("ro-RO")}</span></div><button onClick={() => rollback(snapshot)}><RotateCcw size={14} /> Restaurează</button></div>)}{!snapshots.length && <EmptyPanel icon={<Clock3 size={25} />} title="Niciun snapshot" body="Creează primul punct de revenire pentru proiect." />}</div>
          </section>
        )}

        {view === "settings" && <SettingsPanel section={settingsSection} onSection={setSettingsSection} theme={theme} onTheme={changeTheme} density={density} onDensity={changeDensity} onSecretSaved={() => setNotice("Setarea a fost salvată local.")} availableUpdate={availableUpdate} onCheckUpdate={checkForAppUpdate} onInstallUpdate={installAppUpdate} updatingApp={updatingApp} />}
      </main>
      {notice && <div className="toast"><X size={16} /><span>{notice}</span><button onClick={() => setNotice(null)}>Închide</button></div>}
    </div>
  );
}

function Tab({ icon, label, active, badge, dot, onClick }: { icon: React.ReactNode; label: string; active: boolean; badge?: number; dot?: boolean; onClick: () => void }) {
  return <button className={active ? "active" : ""} onClick={onClick}>{icon}{label}{!!badge && <i>{badge}</i>}{dot && <b />}</button>;
}

function Welcome({ onCreate, onOpen }: { onCreate: () => void; onOpen: () => void }) {
  return <section className="welcome"><div className="welcome-orbit"><img src="/nomad-mark.svg" alt="" /></div><span className="welcome-kicker">LOCAL APP STUDIO</span><h1>Construiește local.<br />Păstrează controlul.</h1><p>Un spațiu de lucru Rust pentru idei care devin aplicații, cu fiecare modificare aprobată de tine.</p><div><button onClick={onCreate}><Plus size={17} /> Proiect nou</button><button className="secondary" onClick={onOpen}><FolderOpen size={17} /> Deschide folder</button></div></section>;
}

function EmptyPanel({ icon, title, body }: { icon: React.ReactNode; title: string; body: string }) {
  return <div className="empty-panel">{icon}<h3>{title}</h3><p>{body}</p></div>;
}

function MessageBubble({ message }: { message: ChatMessage }) {
  const visible = message.content.replace(/<nomad-change>[\s\S]*?<\/nomad-change>/g, "").trim();
  return <div className={`message ${message.role}`}><div className="avatar">{message.role === "assistant" ? <Bot size={14} /> : "Tu"}</div><div>{visible || (message.role === "assistant" ? "Am pregătit modificarea de mai jos." : message.content)}</div></div>;
}

function ProposalCard({ proposal, busy, onAccept, onReject }: { proposal: Proposal; busy: boolean; onAccept: (item: Proposal) => void; onReject: (item: Proposal) => void }) {
  const lines = useMemo(() => createLineDiff(proposal.original_content, proposal.proposed_content), [proposal]);
  return <div className="proposal-card"><div className="proposal-head"><div><span>Modificare propusă</span><strong>{proposal.summary}</strong><small>{proposal.path}</small></div><div><button className="reject" onClick={() => onReject(proposal)}><X size={14} /> Respinge</button><button className="accept" disabled={busy} onClick={() => onAccept(proposal)}>{busy ? <LoaderCircle className="spin" size={14} /> : <Check size={14} />} Acceptă</button></div></div><div className="diff"><div className="diff-title"><Code2 size={13} /> Diff · {lines.filter((line) => line.kind === "add").length} adăugate · {lines.filter((line) => line.kind === "remove").length} șterse</div><pre>{lines.slice(0, 220).map((line, index) => <div key={`${index}-${line.kind}`} className={line.kind}><span>{line.oldLine ?? ""}</span><span>{line.newLine ?? ""}</span><em>{line.kind === "add" ? "+" : line.kind === "remove" ? "−" : " "}</em><code>{line.text || " "}</code></div>)}</pre></div></div>;
}

const settingsItems: { id: SettingsSection; label: string; group: string }[] = [
  { id: "general", label: "General", group: "Nomad" },
  { id: "updates", label: "Actualizări", group: "Nomad" },
  { id: "profile", label: "Profil local", group: "Nomad" },
  { id: "ai", label: "Furnizori AI", group: "Construire" },
  { id: "workflow", label: "Workflow", group: "Construire" },
  { id: "tools", label: "Unelte și MCP", group: "Construire" },
  { id: "integrations", label: "Integrări și deploy", group: "Construire" },
  { id: "permissions", label: "Permisiuni agent", group: "Control" },
  { id: "advanced", label: "Avansat", group: "Control" },
  { id: "experiments", label: "Experimente", group: "Control" },
  { id: "help", label: "Ajutor și diagnostic", group: "Suport" },
  { id: "danger", label: "Zonă periculoasă", group: "Suport" },
];

function SettingsPanel({ section, onSection, theme, onTheme, density, onDensity, onSecretSaved, availableUpdate, onCheckUpdate, onInstallUpdate, updatingApp }: { section: SettingsSection; onSection: (value: SettingsSection) => void; theme: Theme; onTheme: (value: Theme) => void; density: Density; onDensity: (value: Density) => void; onSecretSaved: () => void; availableUpdate: AppUpdateInfo | null; onCheckUpdate: () => Promise<AppUpdateInfo | null>; onInstallUpdate: () => Promise<void>; updatingApp: boolean }) {
  const selected = settingsItems.find((item) => item.id === section)!;
  return <section className="settings-layout"><SettingsNavigation active={section} onSelect={onSection} /><div className="settings-content"><header><span>Setări / {selected.label}</span><h1>{selected.label}</h1><p>{settingsDescriptions[section]}</p></header>{section === "updates" ? <UpdateSettings availableUpdate={availableUpdate} onCheckUpdate={onCheckUpdate} onInstallUpdate={onInstallUpdate} updatingApp={updatingApp} /> : section === "ai" ? <AiProvidersSettings onSecretSaved={onSecretSaved} /> : section === "general" ? <GeneralSettings theme={theme} onTheme={onTheme} density={density} onDensity={onDensity} /> : section === "profile" ? <ProfileSettings /> : <FullSettingsSection section={section} />}</div></section>;
}

const settingsDescriptions: Record<SettingsSection, string> = {
  general: "Aspectul aplicației și preferințele păstrate pe acest calculator.",
  updates: "Alege sursa release-urilor și verifică actualizările Nomad.",
  profile: "Un profil local pentru identificarea acestui dispozitiv.",
  ai: "Configurează un API, un CLI autentificat sau un model local.",
  workflow: "Instrucțiuni pentru agent și modul de lucru în proiecte.",
  tools: "Uneltele agentului și serverele MCP configurate local.",
  integrations: "Servicii externe, cloud și opțiuni de publicare.",
  permissions: "Vezi ce poate face agentul și unde cere confirmarea ta.",
  advanced: "Valori implicite pentru proiecte, preview și diagnosticare.",
  experiments: "Funcții aflate în lucru. Opțiunile apar aici când sunt gata de folosit.",
  help: "Pregătește o întrebare sau un raport pentru echipa Nomad.",
  danger: "Exportă preferințele sau gestionează datele sensibile.",
};

function GeneralSettings({ theme, onTheme, density, onDensity }: { theme: Theme; onTheme: (value: Theme) => void; density: Density; onDensity: (value: Density) => void }) {
  return <div className="settings-card"><h2>Aspect și date</h2><p>Preferințele Nomad și istoricul proiectelor sunt păstrate pe acest calculator.</p><SettingSelect title="Temă aplicație" value={theme} options={[{ value: "forest", label: "Pădure" }, { value: "ocean", label: "Ocean" }, { value: "violet", label: "Violet" }]} onChange={(value) => onTheme(value as Theme)} /><SettingSelect title="Densitatea interfeței" value={density} options={[{ value: "comfortable", label: "Confortabilă" }, { value: "compact", label: "Compactă" }]} onChange={(value) => onDensity(value as Density)} /><SettingsStatus title="Limbă interfață" value="Română" description="Limba disponibilă în această versiune." /><SettingsStatus title="Telemetrie anonimă" value="Dezactivată" description="Nomad nu colectează și nu trimite evenimente de utilizare." /><div className="settings-callout"><strong>Stocare locală activă</strong><p>Proiecte, conversații și setări în SQLite; cheile API în seiful de credențiale al sistemului.</p></div></div>;
}

function SettingSelect({ title, value, options, onChange }: { title: string; value: string; options: { value: string; label: string }[]; onChange: (value: string) => void }) {
  return <label>{title}<select value={value} onChange={(event) => onChange(event.target.value)}>{options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}</select></label>;
}

function SettingsStatus({ title, value, description }: { title: string; value: string; description: string }) {
  return <div className="settings-status"><span><strong>{title}</strong><small>{description}</small></span><b>{value}</b></div>;
}

function SettingToggle({ settingKey, title, description }: { settingKey: string; title: string; description: string }) {
  const [value, setValue] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { invoke<string | null>("get_app_setting", { key: settingKey }).then((stored) => setValue(stored === "true")).catch((reason) => setError(String(reason))); }, [settingKey]);
  const update = async (next: boolean) => { setValue(next); setError(""); try { await invoke("set_app_setting", { key: settingKey, value: String(next) }); } catch (reason) { setValue(!next); setError(String(reason)); } };
  return <><label className="setting-toggle"><span><strong>{title}</strong><small>{description}</small></span><input aria-label={title} type="checkbox" checked={value} onChange={(event) => void update(event.target.checked)} /></label>{error && <div className="settings-error">{error}</div>}</>;
}

const sectionConfig: Record<Exclude<SettingsSection, "general" | "updates" | "profile" | "ai">, { title: string; intro: string; fields: { key: string; label: string; placeholder: string }[]; toggles: { key: string; label: string; hint: string }[]; note: string }> = {
  workflow: { title: "Workflow agent", intro: "Scrie instrucțiuni care se aplică conversațiilor noi. Schimbările propuse rămân în așteptare până le accepți.", fields: [{ key: "workflow_instructions", label: "Instrucțiuni suplimentare pentru agent", placeholder: "Exemplu: păstrează componentele mici, folosește TypeScript și explică pe scurt schimbările…" }], toggles: [], note: "Instrucțiunile sunt incluse în contextul agentului. Snapshot-urile Git se creează înainte de aplicarea propunerilor acceptate." },
  tools: { title: "Unelte și MCP", intro: "Adaugă servere MCP pentru o etapă viitoare și configurează limita de timp pentru comenzile rulate în workspace.", fields: [{ key: "mcp_servers_json", label: "Servere MCP (format JSON)", placeholder: "[]" }, { key: "tool_command_timeout", label: "Durată maximă comandă (secunde)", placeholder: "120" }], toggles: [], note: "Timeout-ul este aplicat comenzilor run_command. Registrul MCP este validat și salvat local; conectarea la servere MCP nu este încă activă în această versiune." },
  integrations: { title: "Cloud, integrări și deploy", intro: "Configurează și verifică un endpoint de serviciu compatibil Nomad.", fields: [{ key: "cloud_api_url", label: "Nomad Cloud API", placeholder: "https://api.exemplu.ro" }], toggles: [], note: "Verificarea /health este activă. Autentificarea, sincronizarea, billing-ul și deploy-ul necesită un backend și credențiale configurate." },
  permissions: { title: "Permisiuni agent", intro: "Permisiunile de siguranță active pentru toate proiectele locale.", fields: [], toggles: [], note: "Aceste protecții sunt obligatorii și nu pot fi dezactivate din setări." },
  advanced: { title: "Avansat", intro: "Setări locale folosite ca valori implicite pentru proiectele deschise ulterior.", fields: [{ key: "default_dev_command", label: "Comandă dev server implicită", placeholder: "npm run dev -- --host 127.0.0.1" }, { key: "default_preview_url", label: "Preview implicit", placeholder: "http://127.0.0.1:5173" }, { key: "external_database_host", label: "Gazdă DB externă (opțional, doar hostname)", placeholder: "ep-example.region.aws.neon.tech" }], toggles: [], note: "Proiectele și conversațiile rămân în SQLite local. Gazda DB este doar notiță de configurare; această versiune nu migrează datele." },
  experiments: { title: "Experimente", intro: "Noutăți în dezvoltare, activate numai după ce sunt implementate și testate complet.", fields: [], toggles: [], note: "Momentan nu sunt experimente disponibile. Pagina se va popula odată cu funcții experimentale livrate în aplicație." },
  help: { title: "Ajutor și diagnostic", intro: "Pregătește un raport local și deschide mesajul în aplicația ta de e-mail pentru verificare.", fields: [{ key: "support_contact", label: "E-mail echipă suport", placeholder: "suport@domeniul-tau.ro" }, { key: "support_notes", label: "Detalii raport", placeholder: "Descrie eroarea și pașii care au dus la ea" }], toggles: [], note: "Raportul nu include proiecte, chei API sau loguri în mod automat." },
  danger: { title: "Zonă periculoasă", intro: "Exportă preferințele locale sau șterge valorile sensibile ale aplicației.", fields: [], toggles: [], note: "Ștergerea tuturor datelor locale ar necesita confirmare explicită și va fi adăugată într-un flux separat." },
};

function FullSettingsSection({ section }: { section: Exclude<SettingsSection, "general" | "updates" | "profile" | "ai"> }) {
  const config = sectionConfig[section];
  const [values, setValues] = useState<Record<string, string>>({}); const [status, setStatus] = useState(""); const [checking, setChecking] = useState(false);
  useEffect(() => { void Promise.all(config.fields.map(async ({ key }) => [key, await invoke<string | null>("get_app_setting", { key })] as const)).then((pairs) => setValues(Object.fromEntries(pairs.map(([key, value]) => [key, value ?? ""])))).catch((error) => setStatus(String(error))); }, [section]);
  const save = async () => { try { for (const { key } of config.fields) await invoke("set_app_setting", { key, value: values[key] ?? "" }); setStatus("Setările au fost salvate local."); } catch (error) { setStatus(String(error)); } };
  const exportPrefs = async () => { try { const keys = new Set(["theme", "language", "profile_name", "ai_endpoint", "openai_model", "ai_provider", "active_custom_ai_provider", "custom_ai_providers", "update_endpoint", ...Object.values(sectionConfig).flatMap((item) => [...item.fields.map((field) => field.key), ...item.toggles.map((toggle) => toggle.key)])]); const pairs = await Promise.all([...keys].map(async (key) => [key, await invoke<string | null>("get_app_setting", { key })] as const)); const safeSettings = Object.fromEntries(pairs.filter((pair): pair is readonly [string, string] => pair[1] !== null)); const blob = new Blob([JSON.stringify({ exportedAt: new Date().toISOString(), settings: safeSettings, note: "Cheile și tokenurile secrete au fost omise." }, null, 2)], { type: "application/json" }); const url = URL.createObjectURL(blob); const anchor = document.createElement("a"); anchor.href = url; anchor.download = "nomad-settings-export.json"; anchor.click(); URL.revokeObjectURL(url); setStatus("Exportul preferințelor a fost descărcat; secretele au fost omise."); } catch (error) { setStatus(String(error)); } };
  return <div className="settings-card"><h2>{config.title}</h2><p>{config.intro}</p>{section === "permissions" && <div className="permission-list"><PermissionRow name="read_file · list_files · search_files · git_diff" detail="Citire și inspecție în dosarul proiectului" mode="Doar citire" /><PermissionRow name="write_file · apply_patch" detail="Modificările apar întâi ca propuneri; aplicarea cere acceptarea ta" mode="Aprobare necesară" /><PermissionRow name="run_command" detail="Execută în proiect, după confirmarea comenzii" mode="Confirmare necesară" /><PermissionRow name="Dev server" detail="Pornirea și oprirea serverului local sunt controlate din interfață" mode="Control manual" /><div className="settings-callout"><strong>Izolare locală</strong><p>Uneltele sunt restrânse la workspace; căile cu traversal sau symlink-uri în afara proiectului sunt blocate. Secretele uzuale sunt excluse din contextul AI.</p></div></div>}{config.fields.map((field) => <label key={field.key}>{field.label}{field.key.includes("json") || field.key.includes("instructions") || field.key.includes("notes") ? <textarea rows={field.key.includes("json") ? 7 : 4} spellCheck={field.key.includes("json") ? false : undefined} value={values[field.key] ?? ""} placeholder={field.placeholder} onChange={(event) => setValues((current) => ({ ...current, [field.key]: event.target.value }))} /> : <input inputMode={field.key.includes("timeout") ? "numeric" : undefined} value={values[field.key] ?? ""} placeholder={field.placeholder} onChange={(event) => setValues((current) => ({ ...current, [field.key]: event.target.value }))} />}</label>)}{config.toggles.map((toggle) => <SettingToggle key={toggle.key} settingKey={toggle.key} title={toggle.label} description={toggle.hint} />)}{section === "integrations" && <button className="secondary" disabled={checking || !values.cloud_api_url?.trim()} onClick={async () => { setChecking(true); try { const base = values.cloud_api_url.trim().replace(/\/$/, ""); const response = await fetch(`${base}/health`); if (!response.ok) throw new Error(`HTTP ${response.status}`); const health = await response.json() as { service?: string; billingConfigured?: boolean }; setStatus(`API conectat (${health.service ?? "serviciu"}); billing ${health.billingConfigured ? "configurat" : "neconfigurat"}.`); } catch (error) { setStatus(`Conexiunea nu a reușit: ${String(error)}`); } finally { setChecking(false); } }}><RefreshCw size={15} />{checking ? "Se verifică…" : "Verifică serviciul cloud"}</button>}{section === "help" && <button className="secondary" onClick={() => { const email = (values.support_contact ?? "").trim(); if (!email.includes("@")) { setStatus("Completează o adresă de suport validă înainte să compui raportul."); return; } const subject = encodeURIComponent("Raport Nomad"); const body = encodeURIComponent(values.support_notes ?? ""); window.location.href = `mailto:${email}?subject=${subject}&body=${body}`; setStatus("Mesajul a fost pregătit în aplicația de e-mail."); }}>Compune raport de suport</button>}{section === "danger" && <><button className="secondary" onClick={() => void exportPrefs()}><Save size={15} /> Exportă preferințele</button><button className="secondary" onClick={async () => { if (!window.confirm("Ștergi cheia API salvată în seiful sistemului? Această acțiune nu poate fi anulată.")) return; await invoke("delete_api_key"); setStatus("Cheia API a fost eliminată din seiful sistemului."); }}>Șterge cheia API din seiful sistemului</button></>}{config.fields.length > 0 && <button onClick={() => void save()}><Save size={15} /> Salvează setările</button>}{config.note && <div className="settings-note">{config.note}</div>}{status && <div role="status" className="settings-note">{status}</div>}</div>;
}

function PermissionRow({ name, detail, mode }: { name: string; detail: string; mode: string }) {
  return <div className="permission-row"><span><strong>{name}</strong><small>{detail}</small></span><b>{mode}</b></div>;
}

function UpdateSettings({ availableUpdate, onCheckUpdate, onInstallUpdate, updatingApp }: { availableUpdate: AppUpdateInfo | null; onCheckUpdate: () => Promise<AppUpdateInfo | null>; onInstallUpdate: () => Promise<void>; updatingApp: boolean }) {
  const [endpoint, setEndpoint] = useState("");
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => { invoke<string | null>("get_app_setting", { key: "update_endpoint" }).then((value) => setEndpoint(value ?? "")).catch(() => {}); }, []);
  const save = async () => {
    setBusy(true); setStatus("");
    try { await invoke("set_app_setting", { key: "update_endpoint", value: endpoint.trim() }); setStatus("Feed-ul de actualizări a fost salvat pe acest calculator."); }
    catch (error) { setStatus(String(error)); }
    finally { setBusy(false); }
  };
  const check = async () => {
    setBusy(true); setStatus("");
    try { const update = await onCheckUpdate(); setStatus(update ? `Versiunea ${update.version} este disponibilă.` : "Nomad este la zi."); }
    catch (error) { setStatus(String(error)); }
    finally { setBusy(false); }
  };
  return <div className="settings-card update-settings"><div className="settings-icon"><RefreshCw size={21} /></div><h2>Actualizări Nomad</h2><p>După configurarea feed-ului, Nomad verifică automat la pornire și te anunță când apare o versiune nouă. Actualizările sunt verificate cu semnătură înainte de instalare.</p><label>Feed HTTPS pentru release-uri<input value={endpoint} onChange={(event) => setEndpoint(event.target.value)} placeholder="https://github.com/cont/nomad/releases/latest/download/latest.json" /></label><button onClick={save} disabled={busy}><Save size={15} />Salvează feed-ul</button><button className="secondary" onClick={check} disabled={busy}>{busy ? <LoaderCircle className="spin" size={14} /> : <Search size={14} />}Verifică acum</button>{availableUpdate && <div className="settings-callout"><strong>Versiunea {availableUpdate.version} este disponibilă</strong>{availableUpdate.notes && <p>{availableUpdate.notes}</p>}<button className="primary-small" disabled={updatingApp} onClick={() => void onInstallUpdate()}>{updatingApp ? "Se instalează…" : "Actualizează și repornește"}</button></div>}{status && <div className="settings-note">{status}</div>}<div className="settings-note">Publicarea update-urilor necesită un repository GitHub, release assets și secretul de semnare Tauri. Vezi RELEASE_GUIDE.md.</div></div>;
}

function AiProvidersSettings({ onSecretSaved }: { onSecretSaved: () => void }) {
  const [provider, setProvider] = useState("api");
  const [endpoint, setEndpoint] = useState("https://api.openai.com");
  const [model, setModel] = useState("gpt-6-astra");
  const [key, setKey] = useState("");
  const [hasKey, setHasKey] = useState(false);
  const [tools, setTools] = useState<AiCliStatus[]>([]);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    Promise.all([
      invoke<boolean>("has_api_key"),
      invoke<string | null>("get_app_setting", { key: "ai_endpoint" }),
      invoke<string | null>("get_app_setting", { key: "openai_model" }),
      invoke<string | null>("get_app_setting", { key: "ai_provider" }),
      invoke<AiCliStatus[]>("detect_ai_cli_tools"),
    ]).then(([present, address, currentModel, active, statuses]) => {
      setHasKey(present);
      if (address) setEndpoint(address);
      if (currentModel) setModel(currentModel);
      if (active) setProvider(active);
      setTools(statuses);
    }).catch((reason) => setError(String(reason)));
  }, []);
  const save = async () => {
    setError(null);
    try {
      if (provider === "api" && key.trim()) {
        await invoke("save_api_key", { apiKey: key.trim() });
        setKey("");
        setHasKey(true);
        onSecretSaved();
      }
      await invoke("set_app_setting", { key: "ai_provider", value: provider });
      if (provider === "api") {
        await invoke("set_app_setting", { key: "ai_endpoint", value: endpoint.trim() });
        await invoke("set_app_setting", { key: "openai_model", value: model.trim() });
      }
      setSaved(true);
      window.setTimeout(() => setSaved(false), 2000);
    } catch (reason) { setError(String(reason)); }
  };
  const deleteKey = async () => {
    if (!window.confirm("Ștergi cheia API salvată în seiful sistemului?")) return;
    try { await invoke("delete_api_key"); setHasKey(false); onSecretSaved(); }
    catch (reason) { setError(String(reason)); }
  };
  const selectedTool = tools.find((item) => item.id === provider);
  return <><div className="settings-card"><div className="settings-icon"><KeyRound size={21} /></div><h2>Alege cum te conectezi</h2><p>Configurează un API, un endpoint compatibil OpenAI sau autentificarea CLI locală.</p><label>Tip de furnizor<select value={provider} onChange={(event) => setProvider(event.target.value)}><option value="api">API standard (OpenAI, OpenRouter, Ollama)</option><option value="custom">Provider API personalizat</option><option value="claude-code">Claude Code CLI · contul tău Claude</option><option value="codex">Codex CLI · contul tău OpenAI</option></select></label>{provider === "custom" ? <CustomAiProvidersSettings onActivate={async (id) => { await invoke("set_app_setting", { key: "active_custom_ai_provider", value: id }); await invoke("set_app_setting", { key: "ai_provider", value: "custom" }); setProvider("custom"); setSaved(true); window.setTimeout(() => setSaved(false), 2000); }} /> : provider === "api" ? <><div className="provider-presets"><span>Setare rapidă endpoint</span><div><button type="button" className="secondary" onClick={() => setEndpoint("https://api.openai.com")}>OpenAI</button><button type="button" className="secondary" onClick={() => setEndpoint("https://openrouter.ai/api/v1")}>OpenRouter</button><button type="button" className="secondary" onClick={() => setEndpoint("http://localhost:11434/v1")}>Ollama local</button></div></div><label>Endpoint<input value={endpoint} onChange={(event) => setEndpoint(event.target.value)} placeholder="https://openrouter.ai/api/v1" /></label><label>Cheie API <span>{hasKey ? "Configurată în seiful sistemului" : "Lipsește · opțional pentru localhost"}</span><input type="password" autoComplete="new-password" value={key} onChange={(event) => setKey(event.target.value)} placeholder={hasKey ? "•••••••• (introdu alta pentru înlocuire)" : "Cheie API"} /></label><label>Model<input value={model} onChange={(event) => setModel(event.target.value)} placeholder="provider/model sau numele modelului" /></label><div className="settings-callout"><strong>Surse și opțiuni gratuite</strong><p>Pentru OpenRouter, alege un model marcat free în catalogul lor. Pentru Ollama, lasă cheia goală și rulează modelul local. Costurile și limitele depind de furnizor.</p>{tools.find((item) => item.id === "ollama")?.available ? <p>Ollama instalat: {tools.find((item) => item.id === "ollama")?.executable}</p> : <a href="https://ollama.com/download" target="_blank" rel="noreferrer">Descarcă Ollama pentru rulare locală</a>}</div></> : <div className="settings-callout"><strong>{selectedTool?.available ? `Găsit: ${selectedTool.executable}` : "CLI negăsit în PATH"}</strong><p>{provider === "codex" ? "Instalează Codex CLI și autentifică-te cu propriul cont ChatGPT/Codex. Cererile Nomad rulează read-only; modificările sunt propuneri pe care le aprobi în Nomad." : "Instalează Claude Code CLI și autentifică-te cu propriul cont Claude Pro/Max sau Console. Cererile rulează în plan mode, cu limită de un tur, fără aplicarea modificărilor."}</p>{selectedTool && <a href={selectedTool.help_url} target="_blank" rel="noreferrer">Instrucțiuni oficiale de instalare și autentificare</a>}{provider === "codex" && <a href="https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server" target="_blank" rel="noreferrer">Detalii despre folosirea planului Codex</a>}</div>}{error && <div className="settings-error">{error}</div>}{provider !== "custom" && <button onClick={() => void save()}>{saved ? <Check size={15} /> : <Save size={15} />}{saved ? "Salvat" : "Salvează și activează"}</button>}{provider === "api" && hasKey && <button className="secondary" onClick={() => void deleteKey()}>Șterge cheia API din seiful sistemului</button>}<div className="settings-note"><Search size={14} /> Cheile API sunt stocate în seiful sistemului. Providerii custom își păstrează cheile separat în seiful sistemului.</div></div><div className="settings-callout settings-wide"><strong>Utilizare simultană</strong><p>Nomad folosește un provider activ pe conversație. Schimbarea providerului afectează conversațiile noi.</p></div></>;
}

function CustomAiProvidersSettings({ onActivate }: { onActivate: (id: string) => Promise<void> }) {
  const [providers, setProviders] = useState<CustomAiProvider[]>([]);
  const [activeId, setActiveId] = useState("");
  const [keys, setKeys] = useState<Record<string, boolean>>({});
  const [draft, setDraft] = useState({ id: "", name: "", baseUrl: "", model: "", protocol: "openai-compatible" as ApiProtocol, apiKeyEnv: "", apiKey: "" });
  const [editingId, setEditingId] = useState<string | null>(null);
  const [selectedTemplate, setSelectedTemplate] = useState("custom");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const loadProviders = useCallback(async () => {
    const [raw, selected] = await Promise.all([
      invoke<string | null>("get_app_setting", { key: "custom_ai_providers" }),
      invoke<string | null>("get_app_setting", { key: "active_custom_ai_provider" }),
    ]);
    const stored = raw ? JSON.parse(raw) as (Partial<CustomAiProvider> & Pick<CustomAiProvider, "id" | "name" | "baseUrl" | "model" | "apiKeyEnv">)[] : [];
    const list: CustomAiProvider[] = stored.map((item) => ({ ...item, protocol: item.protocol ?? "openai-compatible" }));
    setProviders(list);
    setActiveId(selected ?? "");
    const keyPairs = await Promise.all(list.map(async (item) => [item.id, await invoke<boolean>("has_custom_api_key", { providerId: item.id })] as const));
    setKeys(Object.fromEntries(keyPairs));
  }, []);
  useEffect(() => { void loadProviders().catch((reason) => setError(String(reason))); }, [loadProviders]);
  const openNew = () => {
    setEditingId(null);
    setDraft({ id: "", name: "", baseUrl: "", model: "", protocol: "openai-compatible", apiKeyEnv: "", apiKey: "" });
    setSelectedTemplate("custom");
    setError("");
    setDialogOpen(true);
  };
  const openEdit = (item: CustomAiProvider) => {
    setEditingId(item.id);
    setDraft({ ...item, apiKey: "" });
    setSelectedTemplate(aiProviderTemplates.find((template) => template.provider.id === item.id)?.id ?? "custom");
    setError("");
    setDialogOpen(true);
  };
  const saveProvider = async () => {
    setBusy(true);
    setError("");
    try {
      const item: CustomAiProvider = {
        id: editingId ?? draft.id.trim().toLowerCase(),
        name: draft.name.trim(),
        baseUrl: draft.baseUrl.trim().replace(/\/$/, ""),
        model: draft.model.trim(),
        protocol: draft.protocol,
        apiKeyEnv: draft.apiKeyEnv.trim(),
      };
      if (!item.id || !/^[a-z0-9][a-z0-9_-]{0,39}$/.test(item.id)) throw new Error("ID-ul trebuie să înceapă cu literă sau cifră și să conțină doar litere mici, cifre, cratimă sau underscore.");
      if (!item.name || !item.baseUrl || !item.model) throw new Error("Completează numele afișat, URL-ul API și modelul.");
      if (!editingId && providers.some((current) => current.id === item.id)) throw new Error("Există deja un provider cu acest ID.");
      const next = editingId
        ? providers.map((current) => current.id === editingId ? item : current)
        : [...providers, item];
      await invoke("set_app_setting", { key: "custom_ai_providers", value: JSON.stringify(next) });
      if (draft.apiKey.trim()) await invoke("save_custom_api_key", { providerId: item.id, apiKey: draft.apiKey.trim() });
      setProviders(next);
      if (draft.apiKey.trim()) setKeys((current) => ({ ...current, [item.id]: true }));
      setDialogOpen(false);
      setStatus(`„${item.name}” a fost salvat.`);
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };
  const activate = async (item: CustomAiProvider) => {
    setBusy(true); setError(""); setStatus("");
    try {
      await onActivate(item.id);
      setActiveId(item.id);
      setStatus(`„${item.name}” este providerul activ pentru conversațiile noi.`);
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };
  const testProvider = async (item: CustomAiProvider) => {
    setBusy(true); setError(""); setStatus("");
    try { setStatus(await invoke<string>("test_custom_ai_provider", { providerId: item.id })); }
    catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };
  const removeProvider = async (item: CustomAiProvider) => {
    if (!window.confirm(`Ștergi providerul „${item.name}” și cheia lui API salvată local?`)) return;
    setBusy(true); setError("");
    try {
      const next = providers.filter((current) => current.id !== item.id);
      await invoke("delete_custom_api_key", { providerId: item.id });
      await invoke("set_app_setting", { key: "custom_ai_providers", value: JSON.stringify(next) });
      setProviders(next);
      setKeys((current) => { const copy = { ...current }; delete copy[item.id]; return copy; });
      if (activeId === item.id) {
        await invoke("set_app_setting", { key: "ai_provider", value: "api" });
        setActiveId("");
      }
      setStatus(`„${item.name}” a fost șters.`);
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };
  return <div className="custom-provider-manager">
    <div className="provider-manager-head"><div><strong>Provideri API</strong><small>OpenAI compatible, Anthropic Messages, Gemini, Azure și Bedrock</small><small>„Testează” trimite o cerere scurtă; pot fi aplicate limitele sau costurile furnizorului.</small></div><button type="button" className="secondary" onClick={openNew}><Plus size={16} /> Adaugă provider</button></div>
    {providers.length === 0 ? <div className="provider-empty"><KeyRound size={20} /><strong>Configurează primul provider</strong><span>Alege un furnizor din șabloane sau introdu valorile manual. Cheia API rămâne în seiful securizat al sistemului.</span><button type="button" onClick={openNew}><Plus size={16} /> Adaugă API personalizat</button></div> : <div className="provider-list">{providers.map((item) => <article className={`provider-card ${activeId === item.id ? "is-active" : ""}`} key={item.id}><div className="provider-card-main"><div className="provider-card-icon"><KeyRound size={18} /></div><div><div className="provider-card-title"><strong>{item.name}</strong>{activeId === item.id && <span>ACTIV</span>}</div><small>{item.baseUrl}</small><small>Model: <code>{item.model}</code></small><small>{item.protocol} · {keys[item.id] ? "Cheie stocată securizat" : item.apiKeyEnv ? `Cheie din ${item.apiKeyEnv} dacă există` : "Cheie API neconfigurată"}</small></div></div><div className="provider-card-actions"><button type="button" className="secondary" disabled={busy} onClick={() => void testProvider(item)}>Testează</button><button type="button" className="secondary" disabled={busy || activeId === item.id} onClick={() => void activate(item)}>{activeId === item.id ? "Activ" : "Folosește"}</button><button type="button" className="secondary" disabled={busy} onClick={() => openEdit(item)}>Editează</button><button type="button" className="provider-remove" title={`Șterge ${item.name}`} disabled={busy} onClick={() => void removeProvider(item)}><X size={16} /></button></div></article>)}</div>}
    {error && <div role="alert" className="settings-error">{error}</div>}{status && <div role="status" className="settings-note">{status}</div>}
    {dialogOpen && <div className="provider-dialog-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget && !busy) setDialogOpen(false); }}><section className="provider-dialog" role="dialog" aria-modal="true" aria-labelledby="provider-dialog-title"><button type="button" className="provider-dialog-close" aria-label="Închide" onClick={() => setDialogOpen(false)}><X size={18} /></button><div className="settings-icon"><Plus size={20} /></div><h2 id="provider-dialog-title">{editingId ? "Editează providerul" : "Adaugă provider API"}</h2><p>Alege un provider din listă pentru a completa automat adresa, formatul API și variabila de cheie. Secretul se păstrează separat în seiful sistemului.</p>{!editingId && <label>Provider<select value={selectedTemplate} onChange={(event) => { const selected = aiProviderTemplates.find((template) => template.id === event.target.value); if (!selected) return; setSelectedTemplate(selected.id); setDraft((current) => ({ ...selected.provider, apiKey: current.apiKey })); }}><option value="custom">Alt API compatibil</option>{aiProviderTemplates.filter((template) => template.id !== "custom").map((template) => <option key={template.id} value={template.id}>{template.label}</option>)}</select></label>}<label>ID provider<input disabled={Boolean(editingId)} value={editingId ?? draft.id} onChange={(event) => setDraft((current) => ({ ...current, id: event.target.value.toLowerCase().replace(/[^a-z0-9_-]/g, "-") }))} placeholder="ex: openrouter" autoFocus /></label><label>Nume afișat<input value={draft.name} onChange={(event) => setDraft((current) => ({ ...current, name: event.target.value }))} placeholder="ex: OpenRouter" /></label><label>URL de bază API<input value={draft.baseUrl} onChange={(event) => setDraft((current) => ({ ...current, baseUrl: event.target.value }))} placeholder={aiProviderTemplates.find((template) => template.id === selectedTemplate)?.endpointHint ?? "https://api.exemplu.com/v1"} /></label><label>Model<input value={draft.model} onChange={(event) => setDraft((current) => ({ ...current, model: event.target.value }))} placeholder={aiProviderTemplates.find((template) => template.id === selectedTemplate)?.modelHint ?? "ID-ul modelului din catalogul providerului"} /></label><label>Cheie API{keys[draft.id] && <small>Există o cheie salvată. Lasă gol pentru a o păstra.</small>}<input type="password" autoComplete="new-password" value={draft.apiKey} onChange={(event) => setDraft((current) => ({ ...current, apiKey: event.target.value }))} placeholder={keys[draft.id] ? "•••••••• · lasă gol pentru a păstra" : "Lipește cheia API"} /><small>Necesară pentru API-urile externe. Ollama local nu cere cheie.</small></label><details className="provider-advanced"><summary>Autentificare și format API</summary><label>Format API<select value={draft.protocol} onChange={(event) => setDraft((current) => ({ ...current, protocol: event.target.value as ApiProtocol }))}><option value="openai-compatible">OpenAI compatible · OpenRouter, xAI, MiniMax, Bedrock</option><option value="anthropic-messages">Anthropic Messages · Anthropic, MiniMax</option><option value="google-gemini">Google Gemini API</option><option value="google-vertex-express">Google Vertex AI Express</option><option value="azure-openai">Azure OpenAI</option></select></label><label>Variabilă de mediu pentru cheie (opțional)<input value={draft.apiKeyEnv} onChange={(event) => setDraft((current) => ({ ...current, apiKeyEnv: event.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, "") }))} placeholder="ex: OPENROUTER_API_KEY" /><small>Valoarea acestei variabile are prioritate față de cheia din seif, dacă este prezentă.</small></label></details>{error && <div role="alert" className="settings-error">{error}</div>}<div className="provider-dialog-actions"><button type="button" className="secondary" disabled={busy} onClick={() => setDialogOpen(false)}>Anulează</button><button type="button" disabled={busy} onClick={() => void saveProvider()}>{busy ? <LoaderCircle size={15} className="spin" /> : <Save size={15} />}{editingId ? "Salvează providerul" : "Adaugă provider"}</button></div></section></div>}
  </div>;
}

function SettingsNavigation({ active, onSelect }: { active: SettingsSection; onSelect: (value: SettingsSection) => void }) {
  const [query, setQuery] = useState("");
  const searchable = [
    "aspect tema densitate locale", "update actualizare release", "profil nume cont",
    "ai api personalizat custom provider openai anthropic claude codex model cheie ollama openrouter", "workflow instructiuni agent snapshot", "unelte mcp timeout servere", "integrari cloud deploy vercel neon supabase", "permisiuni securitate confirmare protectie", "avansat defaults comanda preview baza de date logs", "experimente beta functii noi", "ajutor suport diagnostic eroare feedback", "danger stergere reset export secret cheie",
  ];
  const normalized = query.trim().toLocaleLowerCase("ro");
  const filteredItems = settingsItems.filter((item, index) => `${item.label} ${item.group} ${searchable[index]}`.toLocaleLowerCase("ro").includes(normalized));
  const groups = [...new Set(filteredItems.map((item) => item.group))];
  return <aside className="settings-nav"><h2>Setări</h2><label className="settings-search"><Search size={15} /><input aria-label="Caută în setări" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Caută setări…" />{query && <button title="Șterge căutarea" onClick={() => setQuery("")}><X size={14} /></button>}</label>{groups.map((group) => <div key={group}><small>{group}</small>{filteredItems.filter((item) => item.group === group).map((item) => <button key={item.id} className={active === item.id ? "active" : ""} onClick={() => onSelect(item.id)}>{item.label}</button>)}</div>)}{filteredItems.length === 0 && <p className="settings-no-results">Nicio setare găsită. Încearcă „AI”, „temă” sau „MCP”.</p>}</aside>;
}

function ProfileSettings() {
  const [name, setName] = useState("Local");
  const [saved, setSaved] = useState(false);
  useEffect(() => { invoke<string | null>("get_app_setting", { key: "profile_name" }).then((value) => { if (value) setName(value); }).catch(() => {}); }, []);
  const save = async () => { await invoke("set_app_setting", { key: "profile_name", value: name.trim() }); setSaved(true); window.setTimeout(() => setSaved(false), 1800); };
  return <div className="settings-card"><div className="settings-icon"><Bot size={21} /></div><h2>Profil pe acest dispozitiv</h2><p>Nomad rulează fără cont; acest nume rămâne local și nu activează sincronizarea.</p><label>Numele profilului<input value={name} onChange={(event) => setName(event.target.value)} maxLength={80} /></label><button onClick={() => void save()}>{saved ? <Check size={15} /> : <Save size={15} />}{saved ? "Salvat" : "Salvează local"}</button><div className="settings-callout"><strong>Abonamente și credite</strong><p>Conturi, plan Pro și creditele agentului intern necesită serviciul online Nomad, care nu este configurat încă.</p></div></div>;
}

function SettingsInfo({ icon, title, body, bullets }: { icon: React.ReactNode; title: string; body: string; bullets: string[] }) {
  return <div className="settings-card"><div className="settings-icon">{icon}</div><h2>{title}</h2><p>{body}</p><ul className="settings-bullets">{bullets.map((bullet) => <li key={bullet}>{bullet}</li>)}</ul></div>;
}

export default App;
