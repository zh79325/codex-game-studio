export type Project = {
  id: string;
  name: string;
  code: string | null;
  root: string;
  state: "drafting" | "styleSettled" | "ready";
};

export type AgentAction = {
  action: "ask_user" | "handoff" | "done" | "blocked";
  target_agent: string | null;
  reason: string;
  payload: {
    choices?: Array<{
      item: string;
      options: string[];
      recommended: string[];
      multiple: boolean;
    }>;
    progress?: {
      decisions: string[];
      open_questions: string[];
      next_step: string;
    };
    drafts?: Array<{
      target_path: string;
      content: string;
      based_on_hash?: string | null;
    }>;
    memories?: Array<{ scope: string; kind: string; content: string }>;
    naming?: Array<{ name: string; code: string; reason: string }>;
    asset_specs?: unknown[];
    verdict?: {
      token: "SPEC-CHECK" | "VIEW-CHECK";
      decision: "APPROVE" | "CONCERNS" | "REJECT";
      sections: Record<string, unknown[]>;
      constraints: Array<{ item: string; value: string }>;
    };
    result?: {
      status: "success" | "failed";
      artifacts: Array<Record<string, unknown>>;
      error: string | null;
    };
  };
};

export type Conversation = {
  id: string;
  projectId: string;
  targetKind: "project" | "character";
  targetRef: string | null;
  title: string;
  directorAgentCode: string;
  focusAgentCode: string | null;
  status: "active" | "running";
  turn: number;
  createdAt: number;
  updatedAt: number;
};

export type ConversationMessage = {
  id: string;
  turn: number;
  role: "user" | "assistant";
  content: string;
  agentCode: string;
  recipientAgentCode: string | null;
  status: "thinking" | "completed" | "failed" | "interrupted";
  tokenCount: number;
  folded: boolean;
  attachments: unknown[];
  action: AgentAction | null;
  createdAt: number;
};

export type ArtifactDraft = {
  id: string;
  conversationId: string;
  targetPath: string;
  content: string;
  basedOnHash: string | null;
  status: string;
  createdAt: number;
};

export type ConversationMemory = {
  id: string;
  conversationId: string;
  scope: string;
  kind: string;
  content: string;
  createdAt: number;
};

export type AgentHandoff = {
  id: number;
  conversationId: string;
  turn: number;
  fromAgentCode: string;
  toAgentCode: string;
  source: string;
  reason: string;
  status: string;
  createdAt: number;
};

export type ConversationSnapshot = {
  conversation: Conversation;
  messages: ConversationMessage[];
  drafts: ArtifactDraft[];
  memories: ConversationMemory[];
  handoffs: AgentHandoff[];
};

export type Character = {
  id: string;
  projectId: string;
  name: string;
  group: string | null;
  dirName: string;
  state:
    | "S0_spec_drafting"
    | "S1_spec_confirmed"
    | "S2_render_generated"
    | "S3_render_confirmed"
    | "S4_views_generated"
    | "S5_views_confirmed";
  specPath: string | null;
  renderPath: string | null;
  viewPaths: Record<string, string>;
  hardConstraints: unknown[];
  gateSpecConfirmedAt: number | null;
  gateRenderConfirmedAt: number | null;
  gateViewsConfirmedAt: number | null;
  createdAt: number;
  updatedAt: number;
};

export type Generation = {
  id: string;
  projectId: string;
  targetKind: string;
  targetRef: string;
  stage: "render" | "views";
  variant: string | null;
  filePath: string;
  fileHash: string | null;
  isFinal: boolean;
  source: string;
  taskId: string | null;
  assetSpec: unknown;
  createdAt: number;
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
  authStyle: string;
  priority: number;
  enabled: boolean;
  remark: string;
  hasKey: boolean;
  keyMask: string | null;
  models: AiModel[];
};

export type AiAgent = {
  agentCode: string;
  role: string;
  roleType: "director" | "specialist" | "executor";
  capability: "text" | "t2i" | "i2i" | "vision" | "model3d" | "t2v" | "i2v";
  requiredModelCapability: string;
  focusable: boolean;
  aliases: string[];
  targetKinds: string[];
  stages: string[];
  maxTurns: number;
  conversational: boolean;
  memoryScope: string;
  contextBudget: number;
  maxOutputTokens: number | null;
  outputContract: string;
  allowTools: string[];
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

export type ProviderPresetModel = {
  modelId: string;
  capabilities: string[];
  driver: string;
  apiPath: string;
  limitKind: string;
  defaultPeriod: string;
  paramsJson: string;
  remark: string;
};

export type ProviderPreset = {
  code: string;
  vendor: string;
  plan: string;
  label: string;
  baseUrl: string;
  driver: string;
  authStyle: string;
  keyPrefix: string | null;
  models: ProviderPresetModel[];
};

export type AiAgentBinding = {
  agentCode: string;
  modelIds: string[];
};
