import type {
  GameArtBibleVersion,
  GameConflict,
  GameFocusWorkflow,
  GameReviewReport,
  GameTask,
  GameUserDecision,
} from "../generated/game";

export type Project = {
  id: string;
  name: string;
  root: string;
  state: string;
};

export type FocusSnapshot = {
  workflow: GameFocusWorkflow;
  reviews: GameReviewReport[];
  conflicts: GameConflict[];
  artBibleDraft: string | null;
  decisions: GameUserDecision[];
};

export type WorkspaceSnapshot = {
  focus?: FocusSnapshot;
  tasks: GameTask[];
  versions: GameArtBibleVersion[];
};

export type AiLimit = {
  limitKind: string;
  maxValue: number;
  periodExpr: string;
  groupName: string;
};

export type AiModel = {
  id: string;
  providerCode: string;
  modelId: string;
  displayName: string;
  capabilities: string[];
  driver: string;
  apiPath: string;
  enabled: boolean;
  sortNo: number;
  paramsJson: string;
  remark: string;
  limits: AiLimit[];
};

export type AiProvider = {
  code: string;
  name: string;
  baseUrl: string;
  driver: string;
  priority: number;
  enabled: boolean;
  hasKey: boolean;
  keyMask: string | null;
  models: AiModel[];
};

export type AiAgent = {
  agentCode: string;
  role: string;
  capability: string;
  outputContract: string;
  sourceFile: string;
  modelIds: string[];
};

export type UsageBudget = {
  limitKind: string;
  used: number;
  limit: number;
  periodExpr: string;
  windowKey: string;
  groupName: string;
  source: string;
  exhausted: boolean;
  unlimited: boolean;
};

export type AiModelUsage = {
  providerCode: string;
  providerName: string;
  providerModelId: string;
  modelId: string;
  providerEnabled: boolean;
  enabled: boolean;
  hasKey: boolean;
  agents: string[];
  budgets: UsageBudget[];
  breaker: {
    failureCount: number;
    lastReason: string | null;
    openedAt: number | null;
    retryAt: number | null;
  } | null;
};

export type ModelRecommendation = {
  providerCode: string;
  providerName: string;
  driver: string;
  defaultBaseUrl: string;
  modelId: string;
  displayName: string;
  capabilities: string[];
  recommended: boolean;
  defaultLimits: AiLimit[];
};
