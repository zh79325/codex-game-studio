import { PauseOutlined, SendOutlined } from "@ant-design/icons";
import {
  Alert,
  App,
  Button,
  Card,
  Checkbox,
  Input,
  Select,
  Space,
  Spin,
  Tag,
  Typography,
} from "antd";
import { useMemo, useState } from "react";
import type {
  AiAgent,
  ArtifactDraft,
  ConversationMessage,
  ConversationSnapshot,
} from "../types";

export type ChatPanelProps = {
  snapshot?: ConversationSnapshot;
  agents?: AiAgent[];
  loading?: boolean;
  canWrite: boolean;
  streamingText?: string;
  workingAgentCode?: string;
  lastError?: string;
  onSend: (content: string, recipientAgentCode?: string) => Promise<unknown>;
  onInterrupt: () => Promise<unknown>;
  onCommitDrafts?: (draftIds: string[]) => Promise<unknown>;
  renderDraftAction?: (draft: ArtifactDraft) => React.ReactNode;
};

export default function ChatPanel(props: ChatPanelProps) {
  const { message } = App.useApp();
  const [content, setContent] = useState("");
  const [recipient, setRecipient] = useState<string>();
  const [selectedDrafts, setSelectedDrafts] = useState<string[]>([]);
  const busy = props.snapshot?.conversation.status === "running";
  const availableAgents = useMemo(
    () => props.agents?.filter((agent) => agent.focusable) ?? [],
    [props.agents],
  );
  const pendingDrafts =
    props.snapshot?.drafts.filter((draft) => draft.status === "pending") ?? [];
  const committableDraftIds = pendingDrafts
    .filter((draft) => !isDedicatedGateDraft(draft))
    .map((draft) => draft.id);

  const send = async (value = content) => {
    const normalized = value.trim();
    if (!normalized) return;
    try {
      await props.onSend(normalized, recipient);
      setContent("");
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <Card
      className="content-card chat-panel"
      title={
        <Space wrap>
          <span>Action 会话</span>
          <Tag color={busy ? "processing" : "default"}>
            {busy ? "运行中" : "待命"}
          </Tag>
          <Tag color="geekblue">
            对焦{" "}
            {props.snapshot?.conversation.focusAgentCode ??
              props.snapshot?.conversation.directorAgentCode ??
              "studio_director"}
          </Tag>
          {props.workingAgentCode && (
            <Tag color="processing">工作中 {props.workingAgentCode}</Tag>
          )}
        </Space>
      }
      loading={props.loading}
    >
      {props.lastError && (
        <Alert type="error" showIcon message={props.lastError} />
      )}
      <MessageList
        messages={props.snapshot?.messages ?? []}
        streamingText={props.streamingText}
        workingAgentCode={props.workingAgentCode}
        onChoice={send}
        disabled={!props.canWrite || busy}
      />

      {pendingDrafts.length > 0 && (
        <DraftDiffPanel
          drafts={pendingDrafts}
          selected={selectedDrafts}
          onSelected={setSelectedDrafts}
          renderAction={props.renderDraftAction}
          onCommit={
            props.onCommitDrafts && committableDraftIds.length
              ? async () => {
                  try {
                    await props.onCommitDrafts!(
                      selectedDrafts.filter((id) =>
                        committableDraftIds.includes(id),
                      ),
                    );
                    setSelectedDrafts([]);
                  } catch (error) {
                    message.error(
                      error instanceof Error ? error.message : String(error),
                    );
                  }
                }
              : undefined
          }
          disabled={!props.canWrite || busy}
        />
      )}

      <Composer
        value={content}
        recipient={recipient}
        agents={availableAgents}
        busy={busy}
        disabled={!props.canWrite || !props.snapshot}
        onChange={setContent}
        onRecipient={setRecipient}
        onSend={() => void send()}
        onInterrupt={() =>
          void props
            .onInterrupt()
            .catch((error) => message.error(String(error)))
        }
      />
    </Card>
  );
}

export function MessageList({
  messages,
  streamingText,
  workingAgentCode,
  disabled,
  onChoice,
}: {
  messages: ConversationMessage[];
  streamingText?: string;
  workingAgentCode?: string;
  disabled: boolean;
  onChoice: (content: string) => Promise<void>;
}) {
  if (!messages.length) {
    return (
      <div className="chat-empty">
        描述当前素材目标，Agent 会通过 Action 给出下一步。
      </div>
    );
  }
  return (
    <div className="message-list">
      {messages.map((item) => (
        <article
          className={`chat-message chat-message-${item.role}`}
          key={item.id}
        >
          <Space className="message-meta" wrap>
            <Typography.Text strong>
              {item.role === "user" ? "你" : item.agentCode}
            </Typography.Text>
            <Tag>{item.status}</Tag>
          </Space>
          {item.status === "thinking" && !item.content ? (
            streamingText && item.agentCode === workingAgentCode ? (
              <Typography.Paragraph>
                {stripActionBlock(streamingText)}
              </Typography.Paragraph>
            ) : (
              <Spin size="small" />
            )
          ) : (
            <Typography.Paragraph>{item.content}</Typography.Paragraph>
          )}
          {item.action && (
            <div className="action-summary">
              <Tag
                color={
                  item.action.action === "blocked"
                    ? "error"
                    : item.action.action === "handoff"
                      ? "purple"
                      : "success"
                }
              >
                {item.action.action}
              </Tag>
              <Typography.Text type="secondary">
                {item.action.reason}
              </Typography.Text>
              {item.action.payload.choices?.map((group) => (
                <div className="choice-group" key={group.item}>
                  <Typography.Text strong>{group.item}</Typography.Text>
                  <Space wrap>
                    {group.options.map((option) => (
                      <Button
                        key={option}
                        size="small"
                        disabled={disabled}
                        onClick={() =>
                          void onChoice(`${group.item}：${option}`)
                        }
                      >
                        {option}
                        {group.recommended.includes(option) ? "（推荐）" : ""}
                      </Button>
                    ))}
                  </Space>
                </div>
              ))}
            </div>
          )}
        </article>
      ))}
    </div>
  );
}

export function Composer({
  value,
  recipient,
  agents,
  busy,
  disabled,
  onChange,
  onRecipient,
  onSend,
  onInterrupt,
}: {
  value: string;
  recipient?: string;
  agents: AiAgent[];
  busy: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
  onRecipient: (value?: string) => void;
  onSend: () => void;
  onInterrupt: () => void;
}) {
  return (
    <div className="chat-composer">
      <Select
        allowClear
        value={recipient}
        placeholder="自动选择 Agent"
        options={agents.map((agent) => ({
          value: agent.agentCode,
          label: `@${agent.agentCode} · ${agent.role}`,
        }))}
        onChange={onRecipient}
      />
      <Input.TextArea
        value={value}
        autoSize={{ minRows: 3, maxRows: 8 }}
        disabled={disabled || busy}
        placeholder="描述素材要求或回答 Agent 的问题"
        onChange={(event) => onChange(event.target.value)}
        onPressEnter={(event) => {
          if (!event.shiftKey) {
            event.preventDefault();
            if (value.trim()) onSend();
          }
        }}
      />
      <Button
        type={busy ? "default" : "primary"}
        danger={busy}
        icon={busy ? <PauseOutlined /> : <SendOutlined />}
        disabled={disabled || (!busy && !value.trim())}
        onClick={busy ? onInterrupt : onSend}
      >
        {busy ? "中断" : "发送"}
      </Button>
    </div>
  );
}

export function DraftDiffPanel({
  drafts,
  selected,
  onSelected,
  onCommit,
  renderAction,
  disabled,
}: {
  drafts: ArtifactDraft[];
  selected: string[];
  onSelected: (ids: string[]) => void;
  onCommit?: () => Promise<void>;
  renderAction?: (draft: ArtifactDraft) => React.ReactNode;
  disabled: boolean;
}) {
  return (
    <section className="draft-panel">
      <Typography.Title level={5}>待确认草稿</Typography.Title>
      {drafts.map((draft) => (
        <Card
          size="small"
          key={draft.id}
          title={draft.targetPath}
          extra={renderAction?.(draft)}
        >
          {!isDedicatedGateDraft(draft) && (
            <Checkbox
              checked={selected.includes(draft.id)}
              disabled={disabled || !onCommit}
              onChange={(event) =>
                onSelected(
                  event.target.checked
                    ? [...selected, draft.id]
                    : selected.filter((id) => id !== draft.id),
                )
              }
            >
              选择提交
            </Checkbox>
          )}
          <pre className="document draft-document">{draft.content}</pre>
        </Card>
      ))}
      {onCommit && (
        <Button
          disabled={disabled || !selected.length}
          onClick={() => void onCommit()}
        >
          提交所选草稿
        </Button>
      )}
    </section>
  );
}

function stripActionBlock(value: string) {
  return value.split("<-------- ACTION-START------->", 1)[0].trimEnd();
}

function isDedicatedGateDraft(draft: ArtifactDraft) {
  return (
    draft.targetPath === "art-bible.md" ||
    draft.targetPath === "docs/角色定稿.md"
  );
}
