import { app, BrowserWindow, dialog, ipcMain, Notification } from "electron";
import { accessSync, constants, existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { BackendSupervisor } from "./backend";
import {
  IPC_BACKEND_STATE,
  IPC_EVENT,
  IPC_NOTIFY,
  IPC_REQUEST,
  IPC_SELECT_DIRECTORY,
  type BackendState,
} from "../shared/ipc";

let window: BrowserWindow | undefined;
let backend: BackendSupervisor | undefined;
let latestState: BackendState = { type: "starting" };

function createBackend(): BackendSupervisor {
  const workspaceRoot = resolveWorkspaceRoot();
  const codexHome = join(workspaceRoot, ".codex-game", "local", "codex-home");
  mkdirSync(codexHome, { recursive: true });
  accessSync(codexHome, constants.W_OK);
  return new BackendSupervisor(
    process.env.CODEX_GAME_APP_SERVER ?? "codex-app-server",
    workspaceRoot,
    codexHome,
  );
}

function resolveWorkspaceRoot(): string {
  const configured = process.env.CODEX_GAME_WORKSPACE;
  if (configured) {
    const root = resolve(configured);
    if (!existsSync(root)) {
      throw new Error(`CODEX_GAME_WORKSPACE 不存在：${root}`);
    }
    return root;
  }

  for (const start of [process.cwd(), app.getAppPath()]) {
    let candidate = resolve(start);
    while (true) {
      if (existsSync(join(candidate, "pnpm-workspace.yaml"))) return candidate;
      const parent = dirname(candidate);
      if (parent === candidate) break;
      candidate = parent;
    }
  }

  throw new Error(
    "无法确定项目工作区。请设置 CODEX_GAME_WORKSPACE，应用不会使用默认用户目录。",
  );
}

function createWindow(): void {
  window = new BrowserWindow({
    width: 1280,
    height: 820,
    minWidth: 960,
    minHeight: 640,
    webPreferences: {
      preload: join(__dirname, "../preload/index.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  });
  const devUrl = process.env.VITE_DEV_SERVER_URL;
  if (devUrl) void window.loadURL(devUrl);
  else
    void window.loadFile(join(import.meta.dirname, "../renderer/index.html"));
  window.webContents.once("did-finish-load", () => {
    window?.webContents.send(IPC_BACKEND_STATE, latestState);
  });
}

app.whenReady().then(() => {
  createWindow();
  try {
    backend = createBackend();
    backend.on("state", (state: BackendState) => {
      latestState = state;
      window?.webContents.send(IPC_BACKEND_STATE, state);
    });
    backend.on("event", (event: unknown) =>
      window?.webContents.send(IPC_EVENT, event),
    );
    backend.start();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    latestState = { type: "incompatible", message };
    window?.webContents.send(IPC_BACKEND_STATE, latestState);
    dialog.showErrorBox("无法启动 Codex Game Studio", message);
  }
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

ipcMain.handle(IPC_REQUEST, (_event, method: string, params?: unknown) => {
  if (!backend) {
    throw new Error(
      latestState.type === "incompatible" ? latestState.message : "后端未启动",
    );
  }
  return backend.request(method, params);
});
ipcMain.handle(IPC_SELECT_DIRECTORY, async (_event, title: string) => {
  const result = await dialog.showOpenDialog(window!, {
    title,
    properties: ["openDirectory", "createDirectory"],
  });
  return result.canceled ? undefined : result.filePaths[0];
});
ipcMain.handle(IPC_NOTIFY, (_event, title: string, body: string) => {
  if (Notification.isSupported()) new Notification({ title, body }).show();
});

app.on("before-quit", () => backend?.stop());
app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});
