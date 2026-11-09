import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Bot, Box, Check, ChevronRight, CircleStop, Clock3, Code2, File,
  FileCode2, Folder, FolderOpen, GitCommitHorizontal, History, KeyRound,
  LoaderCircle, MessageSquareText, Play, Plus, RefreshCw, RotateCcw,
  Save, Search, Settings, Sparkles, TerminalSquare, X,
} from "lucide-react";
import { createLineDiff } from "./lib/diff";
import { splitCommandLine } from "./lib/command";
import { confirmedInvoke } from "./lib/tauri";
import type {
  ChatMessage, FileEntry, ProcessStatus, Project, Proposal, Snapshot,
  StreamDelta, StreamDone, StreamError,
} from "./types";

type View = "agent" | "editor" | "preview" | "history" | "settings";

const emptyProcess: ProcessStatus = { running: false, pid: null, started_at: null, logs: [] };

function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [project, setProject] = useState<Project | null>(null);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState("");
  const [draft, setDraft] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [streaming, setStreaming] = useState("");
  const [activeRequest, setActiveRequest] = useState<string | null>(null);
  const [prompt, setPrompt] = useState("");
  const [view, setView] = useState<View>("agent");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [processStatus, setProcessStatus] = useState<ProcessStatus>(emptyProcess);
  const [devCommand, setDevCommand] = useState("npm run dev -- --host 127.0.0.1");
  const [previewUrl, setPreviewUrl] = useState("http://localhost:5173");
  const [previewKey, setPreviewKey] = useState(0);
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
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
    setSelectedPath(null);
    setFileContent("");
    setDraft("");
    const [nextFiles, nextMessages, nextProposals, command, url] = await Promise.all([
      invoke<FileEntry[]>("list_project_files", { projectId: next.id }),
      invoke<ChatMessage[]>("list_messages", { projectId: next.id }),
      invoke<Proposal[]>("list_proposals", { projectId: next.id }),
      invoke<string | null>("get_project_setting", { projectId: next.id, key: "dev_command" }),
      invoke<string | null>("get_project_setting", { projectId: next.id, key: "preview_url" }),
    ]);
    setFiles(nextFiles);
    setMessages(nextMessages);
    setProposals(nextProposals);
    if (command) setDevCommand(command);
    if (url) setPreviewUrl(url);
  }, []);

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
    const unlisteners = [
      listen<StreamDelta>("chat://delta", ({ payload }) => {
        if (payload.request_id === activeRequestRef.current) {
          setStreaming((value) => value + payload.delta);
        }
      }),
      listen<StreamDone>("chat://done", ({ payload }) => {
        if (payload.request_id !== activeRequestRef.current) return;
        setMessages((value) => [...value, payload.message]);
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
    ];
    return () => { void Promise.all(unlisteners).then((items) => items.forEach((fn) => fn())); };
  }, [showError]);

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
    setMessages((items) => [...items, optimistic]);
    setPrompt("");
    setStreaming("");
    try {
      const requestId = await invoke<string>("chat_stream", {
        projectId: project.id, prompt: value, activePath: selectedPath,
      });
      activeRequestRef.current = requestId;
      setActiveRequest(requestId);
    } catch (error) { showError(error); }
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
        summary: `Modificare manuală pentru ${selectedPath}`,
      });
      setProposals((items) => [proposal, ...items]);
      setView("agent");
    } catch (error) { showError(error); }
  };

  const startServer = async () => {
    if (!project) return;
    try {
      const parts = splitCommandLine(devCommand);
      if (!parts.length) throw new Error("Introdu o comandă dev-server.");
      await Promise.all([
        invoke("set_project_setting", { projectId: project.id, key: "dev_command", value: devCommand }),
        invoke("set_project_setting", { projectId: project.id, key: "preview_url", value: previewUrl }),
      ]);
      await confirmedInvoke(
        "start_dev_server", { projectId: project.id, program: parts[0], args: parts.slice(1) },
        `process:start:${project.id}:${parts.join(" ")}`,
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
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand"><div className="brand-mark"><Sparkles size={17} /></div><span>Nomad</span><em>local</em></div>
        <div className="sidebar-heading"><span>Proiecte</span><div>
          <button className="icon-button" title="Deschide folder" onClick={chooseOpen}><FolderOpen size={15} /></button>
          <button className="icon-button" title="Proiect nou" onClick={chooseCreate}><Plus size={15} /></button>
        </div></div>
        <div className="project-list">
          {projects.map((item) => (
            <button key={item.id} className={`project-item ${project?.id === item.id ? "active" : ""}`} onClick={() => loadProject(item).catch(showError)}>
              <Box size={15} /><span><strong>{item.name}</strong><small>{item.path}</small></span>
            </button>
          ))}
          {!projects.length && <div className="empty-mini">Deschide sau creează primul proiect.</div>}
        </div>
        <div className="sidebar-heading files-heading"><span>Fișiere</span><button className="icon-button" onClick={() => refreshProject().catch(showError)}><RefreshCw size={13} /></button></div>
        <div className="file-tree">
          {files.map((entry) => (
            <button key={entry.path} className={`file-row ${selectedPath === entry.path ? "active" : ""}`} style={{ paddingLeft: 10 + entry.depth * 13 }} onClick={() => selectFile(entry)} disabled={entry.is_dir}>
              {entry.is_dir ? <Folder size={14} /> : <FileCode2 size={14} />}<span>{entry.path.split("/").at(-1)}</span>
            </button>
          ))}
        </div>
        <button className="settings-link" onClick={() => setView("settings")}><Settings size={15} /> Setări</button>
      </aside>

      <main className="workspace">
        <header className="topbar">
          <div className="project-crumb"><span>{project?.name ?? "Niciun proiect"}</span>{selectedPath && <><ChevronRight size={13} /><small>{selectedPath}</small></>}</div>
          <nav>
            <Tab icon={<MessageSquareText size={15} />} label="Agent" active={view === "agent"} badge={pending.length} onClick={() => setView("agent")} />
            <Tab icon={<Code2 size={15} />} label="Editor" active={view === "editor"} onClick={() => setView("editor")} />
            <Tab icon={<Play size={14} />} label="Preview" active={view === "preview"} dot={processStatus.running} onClick={() => setView("preview")} />
            <Tab icon={<History size={15} />} label="Istoric" active={view === "history"} onClick={() => setView("history")} />
          </nav>
        </header>

        {!project && <Welcome onCreate={chooseCreate} onOpen={chooseOpen} />}
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
              <div className="composer-footer"><span>{selectedPath ? `Context: ${selectedPath}` : "Context: proiect"}</span><button onClick={sendPrompt} disabled={!prompt.trim() || !!activeRequest}>{activeRequest ? <LoaderCircle className="spin" size={16} /> : <Sparkles size={16} />} Trimite</button></div>
            </div>
          </section>
        )}

        {project && view === "editor" && (
          <section className="editor-view">
            {selectedPath ? <>
              <div className="panel-toolbar"><div><File size={15} /> {selectedPath}</div><button className="primary-small" disabled={draft === fileContent} onClick={proposeEditorChange}><Save size={14} /> Propune salvarea</button></div>
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
              <button className="icon-button" title="Reîncarcă" onClick={() => setPreviewKey((value) => value + 1)}><RefreshCw size={15} /></button>
              {processStatus.running ? <button className="danger-small" onClick={stopServer}><CircleStop size={14} /> Oprește</button> : <button className="primary-small" onClick={startServer}><Play size={14} /> Pornește</button>}
            </div>
            <div className="command-row"><TerminalSquare size={14} /><input value={devCommand} onChange={(event) => setDevCommand(event.target.value)} /></div>
            <div className="preview-content">
              <iframe key={previewKey} title="Preview aplicație" src={previewUrl} />
              <div className="logs"><div className="logs-title">Loguri proces {processStatus.pid && `· PID ${processStatus.pid}`}</div><pre>{processStatus.logs.slice(-80).join("\n") || "Pornește dev-server-ul pentru a vedea logurile."}</pre></div>
            </div>
          </section>
        )}

        {project && view === "history" && (
          <section className="history-view">
            <div className="history-header"><div><h2>Snapshot-uri Git</h2><p>Puncte locale de revenire, fără remote.</p></div><button className="primary-small" onClick={createSnapshot}><GitCommitHorizontal size={14} /> Snapshot nou</button></div>
            <div className="snapshot-list">{snapshots.map((snapshot) => <div className="snapshot" key={snapshot.hash}><div className="snapshot-dot" /><div><strong>{snapshot.message}</strong><span><code>{snapshot.hash.slice(0, 9)}</code> · {new Date(snapshot.created_at).toLocaleString("ro-RO")}</span></div><button onClick={() => rollback(snapshot)}><RotateCcw size={14} /> Restaurează</button></div>)}{!snapshots.length && <EmptyPanel icon={<Clock3 size={25} />} title="Niciun snapshot" body="Creează primul punct de revenire pentru proiect." />}</div>
          </section>
        )}

        {view === "settings" && <SettingsPanel />}
      </main>
      {notice && <div className="toast"><X size={16} /><span>{notice}</span><button onClick={() => setNotice(null)}>Închide</button></div>}
    </div>
  );
}

function Tab({ icon, label, active, badge, dot, onClick }: { icon: React.ReactNode; label: string; active: boolean; badge?: number; dot?: boolean; onClick: () => void }) {
  return <button className={active ? "active" : ""} onClick={onClick}>{icon}{label}{!!badge && <i>{badge}</i>}{dot && <b />}</button>;
}

function Welcome({ onCreate, onOpen }: { onCreate: () => void; onOpen: () => void }) {
  return <section className="welcome"><div className="welcome-orbit"><Sparkles size={30} /></div><h1>Construiește local.<br />Păstrează controlul.</h1><p>Un spațiu de lucru Rust pentru idei care devin aplicații, cu fiecare modificare aprobată de tine.</p><div><button onClick={onCreate}><Plus size={17} /> Proiect nou</button><button className="secondary" onClick={onOpen}><FolderOpen size={17} /> Deschide folder</button></div></section>;
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

function SettingsPanel() {
  const [key, setKey] = useState("");
  const [model, setModel] = useState("gpt-6-astra");
  const [hasKey, setHasKey] = useState(false);
  const [saved, setSaved] = useState(false);
  useEffect(() => { Promise.all([invoke<boolean>("has_api_key"), invoke<string | null>("get_app_setting", { key: "openai_model" })]).then(([present, value]) => { setHasKey(present); if (value) setModel(value); }); }, []);
  const save = async () => { if (key.trim()) { await invoke("save_api_key", { apiKey: key.trim() }); setHasKey(true); setKey(""); } await invoke("set_app_setting", { key: "openai_model", value: model.trim() }); setSaved(true); window.setTimeout(() => setSaved(false), 2200); };
  return <section className="settings-view"><div className="settings-card"><div className="settings-icon"><KeyRound size={21} /></div><h2>Provider OpenAI</h2><p>Cheia este păstrată în credential vault-ul sistemului. Nomad nu o scrie în SQLite, loguri sau repository.</p><label>Cheie API <span>{hasKey ? "Configurată" : "Lipsește"}</span><input type="password" autoComplete="off" value={key} onChange={(event) => setKey(event.target.value)} placeholder={hasKey ? "•••••••• (introdu alta pentru înlocuire)" : "sk-…"} /></label><label>Model <input value={model} onChange={(event) => setModel(event.target.value)} /></label><button onClick={save}>{saved ? <Check size={15} /> : <Save size={15} />}{saved ? "Salvat" : "Salvează local"}</button><div className="settings-note"><Search size={14} /> Nomad folosește Responses API cu streaming și <code>store: false</code>.</div></div></section>;
}

export default App;
