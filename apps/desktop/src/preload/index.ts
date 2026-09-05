import { contextBridge, ipcRenderer } from "electron";
import {
  IPC_BACKEND_HEARTBEAT,
  IPC_BACKEND_STATE,
  IPC_EVENT,
  IPC_NOTIFY,
  IPC_REQUEST,
  IPC_SELECT_DIRECTORY,
  type BackendState,
  type GameDesktopApi,
} from "../shared/ipc";

const api: GameDesktopApi = {
  request: <T>(method: string, params?: unknown) =>
    ipcRenderer.invoke(IPC_REQUEST, method, params) as Promise<T>,
  heartbeat: () =>
    ipcRenderer.invoke(IPC_BACKEND_HEARTBEAT) as Promise<BackendState>,
  selectDirectory: (title) =>
    ipcRenderer.invoke(IPC_SELECT_DIRECTORY, title) as Promise<
      string | undefined
    >,
  notify: (title, body) =>
    ipcRenderer.invoke(IPC_NOTIFY, title, body) as Promise<void>,
  onEvent: (listener) => {
    const handler = (_event: Electron.IpcRendererEvent, payload: unknown) =>
      listener(payload);
    ipcRenderer.on(IPC_EVENT, handler);
    return () => ipcRenderer.off(IPC_EVENT, handler);
  },
  onBackendState: (listener) => {
    const handler = (_event: Electron.IpcRendererEvent, state: BackendState) =>
      listener(state);
    ipcRenderer.on(IPC_BACKEND_STATE, handler);
    return () => ipcRenderer.off(IPC_BACKEND_STATE, handler);
  },
};

contextBridge.exposeInMainWorld("codexGame", api);
