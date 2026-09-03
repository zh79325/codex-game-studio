import type {
  AiAgent,
  AiModel,
  AiModelUsage,
  AiProvider,
  FocusSnapshot,
  ModelRecommendation,
  Project,
} from "./types";
import type { GameArtBibleVersion, GameTask } from "../generated/game";

export const rpc = <T>(method: string, params: unknown = {}) =>
  window.codexGame.request<T>(method, params);

export const projectsApi = {
  list: async () => (await rpc<{ projects: Project[] }>("game/project/list")).projects,
  create: async (name: string, root: string) =>
    (await rpc<{ project: Project }>("game/project/create", { name, root }))
      .project,
  open: async (root: string) =>
    (await rpc<{ project: Project }>("game/project/open", { root })).project,
  import: async (source: string, destination: string) =>
    rpc<{ project: Project; warnings: string[] }>("game/project/import", {
      source,
      destination,
    }),
};

export const workspaceApi = {
  readProject: async (projectId: string) =>
    (await rpc<{ project: Project }>("game/project/read", { projectId }))
      .project,
  ensureConversation: async (projectId: string) =>
    (
      await rpc<{ conversation: { id: string } }>(
        "game/conversation/ensure",
        { projectId },
      )
    ).conversation,
  startFocus: (conversationId: string) =>
    rpc("game/focus/start", { conversationId }),
  readFocus: (conversationId: string) =>
    rpc<FocusSnapshot>("game/focus/read", { conversationId }),
  listTasks: async (conversationId: string) =>
    (await rpc<{ tasks: GameTask[] }>("game/task/list", { conversationId }))
      .tasks,
  listVersions: async (projectId: string) =>
    (
      await rpc<{ versions: GameArtBibleVersion[] }>("game/artBible/list", {
        projectId,
      })
    ).versions,
};

export const aiApi = {
  listProviders: async () =>
    (await rpc<{ providers: AiProvider[] }>("game/aiProvider/list")).providers,
  writeProvider: async (
    method: "create" | "update",
    provider: AiProvider,
    apiKey?: string,
  ) =>
    (
      await rpc<{ provider: AiProvider }>(`game/aiProvider/${method}`, {
        provider,
        apiKey,
      })
    ).provider,
  deleteProvider: (code: string) =>
    rpc("game/aiProvider/delete", { code }),
  writeModel: async (
    method: "create" | "update",
    model: AiModel,
  ) =>
    (
      await rpc<{ model: AiModel }>(`game/aiModel/${method}`, { model })
    ).model,
  deleteModel: (modelId: string) =>
    rpc("game/aiModel/delete", { modelId }),
  listAgents: async () =>
    (await rpc<{ agents: AiAgent[] }>("game/aiAgent/list")).agents,
  writeAgentBinding: (agentCode: string, modelIds: string[]) =>
    rpc("game/aiAgentBinding/write", { agentCode, modelIds }),
  readUsage: async () =>
    (await rpc<{ items: AiModelUsage[] }>("game/aiUsage/read")).items,
  resetUsage: (modelId: string, limitKind?: string) =>
    rpc<{ cleared: number }>("game/aiUsage/reset", { modelId, limitKind }),
  clearBreaker: (modelId: string) =>
    rpc("game/aiBreaker/clear", { modelId }),
  recommendations: () =>
    rpc<{ recommendations: ModelRecommendation[]; path: string }>(
      "game/modelRecommendation/list",
    ),
  exportConfig: async () =>
    (await rpc<{ json: string }>("game/aiConfig/export")).json,
  importConfig: (json: string, dryRun: boolean) =>
    rpc<{ providerCount: number; modelCount: number; applied: boolean }>(
      "game/aiConfig/import",
      { json, dryRun },
    ),
};
