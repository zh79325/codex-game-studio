import { ChildProcessWithoutNullStreams, spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { EventEmitter } from "node:events";
import type {
  BackendState,
  GamePingResponse,
  JsonRpcRequest,
  JsonRpcResponse,
} from "../shared/ipc";

const GAME_PROTOCOL_VERSION = 1;

export class BackendSupervisor extends EventEmitter {
  private child?: ChildProcessWithoutNullStreams;
  private nextId = 1;
  private restartTimer?: NodeJS.Timeout;
  private readonly pending = new Map<
    number,
    { resolve(value: unknown): void; reject(error: Error): void }
  >();
  private initialized = false;
  private state: BackendState = { type: "starting" };
  private stopping = false;

  constructor(private readonly executable: string) {
    super();
  }

  start(): void {
    this.stopping = false;
    this.spawnBackend();
  }

  stop(): void {
    this.stopping = true;
    if (this.restartTimer) clearTimeout(this.restartTimer);
    this.child?.kill();
    this.child = undefined;
  }

  async request<T>(method: string, params?: unknown): Promise<T> {
    if (!this.child || !this.initialized) {
      throw new Error("后端尚未完成初始化");
    }
    if (this.state.type === "starting" || this.state.type === "recovering") {
      throw new Error("后端尚未完成恢复");
    }
    if (this.state.type === "incompatible" || this.state.type === "stopped") {
      throw new Error(this.state.message);
    }
    if (this.state.type === "readOnly" && !isReadOnlyMethod(method)) {
      throw new Error("项目当前以只读模式打开，不能执行写操作");
    }
    return this.send<T>(method, params);
  }

  private spawnBackend(): void {
    this.emitState({ type: "starting" });
    this.initialized = false;
    const child = spawn(this.executable, ["--listen", "stdio://"], {
      stdio: ["pipe", "pipe", "pipe"],
      env: process.env,
    });
    this.child = child;
    createInterface({ input: child.stdout }).on("line", (line) =>
      this.handleLine(line),
    );
    child.stderr.on("data", (chunk) => this.emit("stderr", chunk.toString()));
    child.on("exit", (_code, signal) => this.handleExit(signal));
    void this.initialize();
  }

  private async initialize(): Promise<void> {
    try {
      await this.send("initialize", {
        clientInfo: {
          name: "codex-game-studio",
          title: "Codex Game Studio",
          version: "0.0.0",
        },
        capabilities: { experimentalApi: true },
      });
      this.initialized = true;
      this.emitState({ type: "recovering" });
      await this.waitUntilReady();
    } catch (error) {
      this.initialized = false;
      this.emitState({
        type: "incompatible",
        message: error instanceof Error ? error.message : String(error),
      });
    }
  }

  private async waitUntilReady(): Promise<void> {
    while (this.child && this.initialized) {
      const ping = await this.send<GamePingResponse>("game/ping", {});
      if (ping.protocolVersion !== GAME_PROTOCOL_VERSION) {
        throw new Error(
          `协议版本不匹配：客户端 ${GAME_PROTOCOL_VERSION}，后端 ${ping.protocolVersion}`,
        );
      }
      if (ping.status === "ready" || ping.status === "readOnly") {
        this.emitState({
          type: ping.status,
          backendVersion: ping.backendVersion,
        });
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }

  private send<T>(method: string, params?: unknown): Promise<T> {
    const child = this.child;
    if (!child) return Promise.reject(new Error("后端进程未运行"));
    const request: JsonRpcRequest = { id: this.nextId++, method, params };
    return new Promise<T>((resolve, reject) => {
      this.pending.set(request.id, {
        resolve: (value) => resolve(value as T),
        reject,
      });
      child.stdin.write(`${JSON.stringify(request)}\n`);
    });
  }

  private handleLine(line: string): void {
    let message: JsonRpcResponse & { method?: string; params?: unknown };
    try {
      message = JSON.parse(line) as typeof message;
    } catch {
      return;
    }
    if (message.method) {
      this.emit("event", { method: message.method, params: message.params });
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) return;
    this.pending.delete(message.id);
    if (message.error) pending.reject(new Error(message.error.message));
    else pending.resolve(message.result);
  }

  private handleExit(signal: NodeJS.Signals | null): void {
    this.initialized = false;
    this.child = undefined;
    for (const pending of this.pending.values()) {
      pending.reject(new Error("后端进程已退出"));
    }
    this.pending.clear();
    if (this.stopping) return;
    this.emitState({ type: "recovering" });
    this.restartTimer = setTimeout(() => this.spawnBackend(), 500);
    if (signal) this.emit("stderr", `backend exited with signal ${signal}`);
  }

  private emitState(state: BackendState): void {
    this.state = state;
    this.emit("state", state);
  }
}

const READ_ONLY_METHODS = new Set([
  "game/ping",
  "game/project/create",
  "game/project/open",
  "game/project/read",
  "game/project/list",
  "game/project/import",
  "game/conversation/read",
  "game/focus/read",
  "game/task/list",
  "game/artBible/list",
  "game/artBible/read",
]);

function isReadOnlyMethod(method: string): boolean {
  return READ_ONLY_METHODS.has(method);
}
