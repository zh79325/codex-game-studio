import type {
  AiAgent,
  AiAgentBinding,
  AiModel,
  AiModelUsage,
  AiProvider,
  Character,
  Conversation,
  ConversationSnapshot,
  Generation,
  Project,
  ProviderPreset,
} from "./types";
import type { GameArtBibleVersion, GameTask } from "../generated/game";

export const rpc = <T>(method: string, params: unknown = {}) =>
  window.codexGame.request<T>(method, params);

export const projectsApi = {
  list: async () =>
    (await rpc<{ projects: Project[] }>("game/project/list")).projects,
  inspect: (root: string) =>
    rpc<{
      root: string;
      occupied: boolean;
      projectId: string | null;
      supported: boolean;
    }>("game/project/inspect", { root }),
  create: async (root: string, overwrite = false) =>
    (
      await rpc<{ project: Project }>("game/project/create", {
        root,
        overwrite,
      })
    ).project,
  open: async (root: string) =>
    (await rpc<{ project: Project }>("game/project/open", { root })).project,
  remove: (projectId: string) =>
    rpc<Record<string, never>>("game/project/delete", { projectId }),
};

export const conversationApi = {
  ensure: async (
    projectId: string,
    targetKind: "project" | "character",
    targetRef: string | null,
    title?: string,
  ) =>
    (
      await rpc<{ conversation: Conversation }>("game/conversation/ensure", {
        projectId,
        targetKind,
        targetRef,
        title,
      })
    ).conversation,
  read: (conversationId: string) =>
    rpc<ConversationSnapshot>("game/conversation/read", { conversationId }),
  send: (
    conversationId: string,
    content: string,
    recipientAgentCode?: string,
  ) =>
    rpc<ConversationSnapshot>("game/conversation/submit", {
      conversationId,
      content,
      recipientAgentCode,
    }),
  interrupt: (conversationId: string) =>
    rpc("game/conversation/interrupt", { conversationId }),
  commitDrafts: (conversationId: string, draftIds: string[]) =>
    rpc("game/conversation/commitDrafts", { conversationId, draftIds }),
};

export const workspaceApi = {
  readProject: async (projectId: string) =>
    (await rpc<{ project: Project }>("game/project/read", { projectId }))
      .project,
  commitArtBible: (conversationId: string, draftId: string) =>
    rpc<{ version: GameArtBibleVersion; markdown: string }>(
      "game/project/commitArtBible",
      { conversationId, draftId },
    ),
  finalize: async (projectId: string, name: string, code: string) =>
    (
      await rpc<{ project: Project }>("game/project/finalize", {
        projectId,
        name,
        code,
      })
    ).project,
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

export const charactersApi = {
  list: async (projectId: string) =>
    (
      await rpc<{ characters: Character[] }>("game/character/list", {
        projectId,
      })
    ).characters,
  create: async (
    projectId: string,
    name: string,
    group: string | null,
    overwrite: boolean,
  ) =>
    (
      await rpc<{ character: Character }>("game/character/create", {
        projectId,
        name,
        group,
        overwrite,
      })
    ).character,
  read: (projectId: string, characterId: string) =>
    rpc<{ character: Character; generations: Generation[] }>(
      "game/character/read",
      {
        projectId,
        characterId,
      },
    ),
  confirmSpec: async (
    projectId: string,
    characterId: string,
    draftId: string,
  ) =>
    (
      await rpc<{ character: Character }>("game/character/confirmSpec", {
        projectId,
        characterId,
        draftId,
      })
    ).character,
  rejectSpec: async (projectId: string, characterId: string, reason: string) =>
    (
      await rpc<{ character: Character }>("game/character/rejectSpec", {
        projectId,
        characterId,
        reason,
      })
    ).character,
  confirmRender: async (
    projectId: string,
    characterId: string,
    generationId: string,
  ) =>
    (
      await rpc<{ character: Character }>("game/character/confirmRender", {
        projectId,
        characterId,
        generationId,
      })
    ).character,
  rejectRender: async (
    projectId: string,
    characterId: string,
    reason: string,
  ) =>
    (
      await rpc<{ character: Character }>("game/character/rejectRender", {
        projectId,
        characterId,
        reason,
      })
    ).character,
  confirmViews: async (
    projectId: string,
    characterId: string,
    generationIds: string[],
  ) =>
    (
      await rpc<{ character: Character }>("game/character/confirmViews", {
        projectId,
        characterId,
        generationIds,
      })
    ).character,
  rejectViews: async (projectId: string, characterId: string, reason: string) =>
    (
      await rpc<{ character: Character }>("game/character/rejectViews", {
        projectId,
        characterId,
        reason,
      })
    ).character,
  listGenerations: async (
    projectId: string,
    characterId: string,
    stage?: string,
  ) =>
    (
      await rpc<{ generations: Generation[] }>("game/generation/list", {
        projectId,
        characterId,
        stage,
      })
    ).generations,
};

export const aiApi = {
  listProviders: async () =>
    (await rpc<{ providers: AiProvider[] }>("game/aiProvider/list")).providers,
  createProvider: async (
    provider: AiProvider,
    apiKey: string | undefined,
    agentBindings: AiAgentBinding[],
  ) =>
    (
      await rpc<{ provider: AiProvider }>("game/aiProvider/create", {
        provider,
        apiKey,
        agentBindings,
      })
    ).provider,
  updateProvider: async (provider: AiProvider, apiKey?: string) =>
    (
      await rpc<{ provider: AiProvider }>("game/aiProvider/update", {
        provider,
        apiKey,
      })
    ).provider,
  deleteProvider: (code: string) => rpc("game/aiProvider/delete", { code }),
  writeModel: async (method: "create" | "update", model: AiModel) =>
    (await rpc<{ model: AiModel }>(`game/aiModel/${method}`, { model })).model,
  deleteModel: (modelId: string) => rpc("game/aiModel/delete", { modelId }),
  listAgents: async () =>
    (await rpc<{ agents: AiAgent[] }>("game/aiAgent/list")).agents,
  writeAgentBinding: (agentCode: string, modelIds: string[]) =>
    rpc("game/aiAgentBinding/write", { agentCode, modelIds }),
  readUsage: async () =>
    (await rpc<{ items: AiModelUsage[] }>("game/aiUsage/read")).items,
  resetUsage: (modelId: string, limitKind?: string) =>
    rpc<{ cleared: number }>("game/aiUsage/reset", { modelId, limitKind }),
  clearBreaker: (modelId: string) => rpc("game/aiBreaker/clear", { modelId }),
  listProviderPresets: () =>
    rpc<{ presets: ProviderPreset[]; path: string }>(
      "game/providerPreset/list",
    ),
  exportConfig: async () =>
    (await rpc<{ json: string }>("game/aiConfig/export")).json,
  importConfig: (json: string, dryRun: boolean) =>
    rpc<{ providerCount: number; modelCount: number; applied: boolean }>(
      "game/aiConfig/import",
      { json, dryRun },
    ),
};
