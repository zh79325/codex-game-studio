import type { GameDesktopApi } from "../shared/ipc";

declare global {
  interface Window {
    codexGame: GameDesktopApi;
  }
}

export {};
