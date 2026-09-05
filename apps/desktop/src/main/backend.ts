import { ChildProcessWithoutNullStreams, spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { EventEmitter } from "node:events";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import type {
  BackendState,
  GamePingResponse,
  JsonRpcRequest,
  JsonRpcResponse,
} from "../shared/ipc";

const GAME_PROTOCOL_VERSION = 1;
const HEARTBEAT_TIMEOUT_MS = 5_000;

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

  constructor(
    private readonly executable: string,
    private readonly workspaceRoot: string,
    private readonly codexHome: string,
  ) {
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
    const result = await this.send<T>(method, params);
    if (RESTART_AFTER_METHODS.has(method)) {
      setTimeout(() => this.restart(), 0);
    }
    return result;
  }

  async heartbeat(): Promise<BackendState> {
    if (!this.child || !this.initialized) return this.state;
    try {
      const ping = await this.send<GamePingResponse>(
        "game/ping",
        {},
        HEARTBEAT_TIMEOUT_MS,
      );
      if (ping.protocolVersion !== GAME_PROTOCOL_VERSION) {
        this.emitState({
          type: "incompatible",
          message: `协议版本不匹配：客户端 ${GAME_PROTOCOL_VERSION}，后端 ${ping.protocolVersion}`,
        });
      } else if (ping.status === "ready" || ping.status === "readOnly") {
        this.emitState({
          type: ping.status,
          backendVersion: ping.backendVersion,
        });
      } else {
        this.emitState({ type: "recovering" });
      }
    } catch {
      if (this.child && this.initialized) {
        this.emitState({ type: "recovering" });
      }
    }
    return this.state;
  }

  private restart(): void {
    if (!this.child) {
      this.spawnBackend();
      return;
    }
    this.initialized = false;
    this.emitState({ type: "recovering" });
    this.child.kill();
  }

  private providerEnvironment(): NodeJS.ProcessEnv {
    const path = join(dirname(this.codexHome), "ai-secrets.json");
    try {
      const parsed = JSON.parse(readFileSync(path, "utf8")) as {
        providerKeys?: Record<string, string>;
      };
      return Object.fromEntries(
        Object.entries(parsed.providerKeys ?? {}).map(([code, key]) => [
          providerKeyEnvironment(code),
          key,
        ]),
      );
    } catch {
      return {};
    }
  }

  private spawnBackend(): void {
    this.emitState({ type: "starting" });
    this.initialized = false;
    const child = spawn(
      this.executable,
      ["--listen", "stdio://", "--disable-plugin-startup-tasks"],
      {
        stdio: ["pipe", "pipe", "pipe"],
        cwd: this.workspaceRoot,
        env: {
          ...process.env,
          ...this.providerEnvironment(),
          CODEX_HOME: this.codexHome,
          CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED: "1",
          RUST_LOG: process.env.RUST_LOG
            ? `${process.env.RUST_LOG},codex_http_client::transport=info`
            : "codex_http_client::transport=info",
        },
      },
    );
    this.child = child;
    createInterface({ input: child.stdout }).on("line", (line) =>
      this.handleLine(line),
    );
    child.stderr.on("data", (chunk) => {
      const message = chunk.toString();
      process.stderr.write(message);
      this.emit("stderr", message);
    });
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
      const state = await this.heartbeat();
      if (
        state.type === "ready" ||
        state.type === "readOnly" ||
        state.type === "incompatible"
      ) {
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }

  private send<T>(
    method: string,
    params?: unknown,
    timeoutMs?: number,
  ): Promise<T> {
    const child = this.child;
    if (!child) return Promise.reject(new Error("后端进程未运行"));
    const request: JsonRpcRequest = { id: this.nextId++, method, params };
    return new Promise<T>((resolve, reject) => {
      let timeout: NodeJS.Timeout | undefined;
      this.pending.set(request.id, {
        resolve: (value) => {
          if (timeout) clearTimeout(timeout);
          resolve(value as T);
        },
        reject: (error) => {
          if (timeout) clearTimeout(timeout);
          reject(error);
        },
      });
      if (timeoutMs) {
        timeout = setTimeout(() => {
          if (!this.pending.delete(request.id)) return;
          reject(new Error(`${method} 请求超时`));
        }, timeoutMs);
      }
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
    if (JSON.stringify(this.state) === JSON.stringify(state)) return;
    this.state = state;
    this.emit("state", state);
  }
}

const RESTART_AFTER_METHODS = new Set([
  "game/aiProvider/create",
  "game/aiProvider/update",
  "game/aiProvider/delete",
  "game/aiModel/create",
  "game/aiModel/update",
  "game/aiModel/delete",
  "game/aiConfig/import",
]);

const READ_ONLY_METHODS = new Set([
  "game/ping",
  "game/project/inspect",
  "game/project/create",
  "game/project/open",
  "game/project/read",
  "game/project/list",
  "game/conversation/read",
  "game/character/list",
  "game/character/read",
  "game/generation/list",
  "game/task/list",
  "game/artBible/list",
  "game/artBible/read",
  "game/aiProvider/list",
  "game/aiAgent/list",
  "game/aiUsage/read",
  "game/providerPreset/list",
  "game/aiConfig/export",
]);

function providerKeyEnvironment(code: string): string {
  return `CODEX_GAME_PROVIDER_${code.replace(/[^A-Za-z0-9]/g, "_").toUpperCase()}_API_KEY`;
}

function isReadOnlyMethod(method: string): boolean {
  return READ_ONLY_METHODS.has(method);
}
