export const IPC_REQUEST = "game:request";
export const IPC_EVENT = "game:event";
export const IPC_BACKEND_STATE = "game:backend-state";
export const IPC_SELECT_DIRECTORY = "native:select-directory";
export const IPC_NOTIFY = "native:notify";

export type JsonRpcId = number;

export interface JsonRpcRequest {
  id: JsonRpcId;
  method: string;
  params?: unknown;
}

export interface JsonRpcResponse {
  id: JsonRpcId;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

export type BackendState =
  | { type: "starting" }
  | { type: "recovering" }
  | { type: "ready"; backendVersion: string }
  | { type: "readOnly"; backendVersion: string }
  | { type: "incompatible"; message: string }
  | { type: "stopped"; message: string };

export interface GamePingResponse {
  protocolVersion: number;
  backendVersion: string;
  status: "starting" | "recovering" | "ready" | "readOnly";
}

export interface GameDesktopApi {
  request<T>(method: string, params?: unknown): Promise<T>;
  selectDirectory(title: string): Promise<string | undefined>;
  notify(title: string, body: string): Promise<void>;
  onEvent(listener: (event: unknown) => void): () => void;
  onBackendState(listener: (state: BackendState) => void): () => void;
}
