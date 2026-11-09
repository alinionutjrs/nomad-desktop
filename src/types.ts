export type Project = {
  id: number;
  name: string;
  path: string;
  created_at: string;
};

export type FileEntry = {
  path: string;
  is_dir: boolean;
  size: number;
  depth: number;
};

export type ChatMessage = {
  id: number;
  role: "user" | "assistant" | "system";
  content: string;
  created_at: string;
};

export type ChatSession = {
  id: number;
  title: string;
  created_at: string;
  updated_at: string;
};

export type Proposal = {
  id: number;
  project_id: number;
  path: string;
  original_content: string;
  proposed_content: string;
  summary: string;
  status: "pending" | "accepted" | "rejected";
  created_at: string;
};

export type Snapshot = {
  hash: string;
  message: string;
  created_at: string;
};

export type ProcessStatus = {
  running: boolean;
  pid: number | null;
  started_at: string | null;
  logs: string[];
};

export type StreamDelta = {
  request_id: string;
  delta: string;
};

export type StreamDone = {
  request_id: string;
  message: ChatMessage;
  proposal: Proposal | null;
};

export type StreamError = {
  request_id: string;
  message: string;
};

