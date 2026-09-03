import { app, BrowserWindow, dialog, ipcMain, Notification } from "electron";
import { join } from "node:path";
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
let latestState: BackendState = { type: "starting" };
const backend = new BackendSupervisor(
  process.env.CODEX_GAME_APP_SERVER ?? "codex-app-server",
);

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
  backend.start();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

backend.on("state", (state: BackendState) => {
  latestState = state;
  window?.webContents.send(IPC_BACKEND_STATE, state);
});
backend.on("event", (event: unknown) =>
  window?.webContents.send(IPC_EVENT, event),
);

ipcMain.handle(IPC_REQUEST, (_event, method: string, params?: unknown) =>
  backend.request(method, params),
);
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

app.on("before-quit", () => backend.stop());
app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});
