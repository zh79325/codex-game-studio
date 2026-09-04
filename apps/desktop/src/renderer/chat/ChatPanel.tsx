import {
  LockOutlined,
  SendOutlined,
  StopOutlined,
  UnlockOutlined,
} from "@ant-design/icons";
import {
  Alert,
  App,
  Button,
  Card,
  Mentions,
  Space,
  Spin,
  Tag,
  Typography,
} from "antd";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type {
  AiAgent,
  ArtifactDraft,
  ConversationMessage,
  ConversationSnapshot,
} from "../types";
import InteractionDrawer from "./InteractionDrawer";

export type ChatPanelProps = {
  snapshot?: ConversationSnapshot;
  agents?: AiAgent[];
  loading?: boolean;
  canWrite: boolean;
  busy: boolean;
  interrupting?: boolean;
  streamingText?: string;
  thinkingText?: string;
  workingAgentCode?: string;
  lastError?: string;
  starterPrompt?: string;
  onSend: (content: string, recipientAgentCode?: string) => Promise<unknown>;
  onInterrupt: () => Promise<unknown>;
  onCommitDrafts?: (draftIds: string[]) => Promise<unknown>;
  renderDraftAction?: (
    draft: ArtifactDraft,
    closeDrawer: () => void,
  ) => React.ReactNode;
};

export default function ChatPanel(props: ChatPanelProps) {
  const { message } = App.useApp();
  const [content, setContent] = useState("");
  const [followLatestRequest, setFollowLatestRequest] = useState(0);
  const availableAgents = useMemo(
    () => props.agents?.filter((agent) => agent.focusable) ?? [],
    [props.agents],
  );
  const pendingDrafts =
    props.snapshot?.drafts.filter((draft) => draft.status === "pending") ?? [];
  const pendingChoice = useMemo(() => {
    const messages = props.snapshot?.messages ?? [];
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const choices = messages[index].action?.payload.choices;
      if (!choices?.length) continue;
      const answered = messages
        .slice(index + 1)
        .some((message) => message.role === "user");
      return answered ? undefined : { id: messages[index].id, groups: choices };
    }
    return undefined;
  }, [props.snapshot?.messages]);

  const send = async (value = content) => {
    const normalized = value.trim();
    if (!normalized) return false;
    setFollowLatestRequest((current) => current + 1);
    try {
      await props.onSend(
        normalized,
        findMentionedAgent(normalized, availableAgents),
      );
      setContent("");
      return true;
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
      return false;
    }
  };

  return (
    <>
      <Card
        className="content-card chat-panel"
        title={
          <Space wrap>
            <span>Action 会话</span>
            <Tag color={props.busy ? "processing" : "default"}>
              {props.busy ? "运行中" : "待命"}
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
          thinkingText={props.thinkingText}
          workingAgentCode={props.workingAgentCode}
          starterPrompt={props.starterPrompt}
          followLatestRequest={followLatestRequest}
          onStarter={send}
          disabled={!props.canWrite || props.busy || !props.snapshot}
        />

        <Composer
          value={content}
          agents={availableAgents}
          busy={props.busy}
          interrupting={props.interrupting}
          disabled={!props.canWrite || !props.snapshot}
          onChange={setContent}
          onSend={() => void send()}
          onInterrupt={() =>
            void props
              .onInterrupt()
              .catch((error) => message.error(String(error)))
          }
        />
      </Card>
      <InteractionDrawer
        choice={pendingChoice}
        drafts={pendingDrafts}
        disabled={!props.canWrite || props.busy}
        onSubmitChoice={send}
        onCommitDrafts={props.onCommitDrafts}
        renderDraftAction={props.renderDraftAction}
      />
    </>
  );
}

function useAutoFollowScroll() {
  const containerRef = useRef<HTMLDivElement>(null);
  const animationFrameRef = useRef<number | undefined>(undefined);
  const programmaticScrollRef = useRef(false);
  const [locked, setLocked] = useState(false);

  const scrollToLatest = useCallback(() => {
    const container = containerRef.current;
    if (!container) return;
    if (animationFrameRef.current !== undefined) {
      window.cancelAnimationFrame(animationFrameRef.current);
    }
    programmaticScrollRef.current = true;
    container.scrollTop = container.scrollHeight;
    animationFrameRef.current = window.requestAnimationFrame(() => {
      programmaticScrollRef.current = false;
      animationFrameRef.current = undefined;
    });
  }, []);

  const lock = useCallback(() => setLocked(true), []);
  const unlock = useCallback(() => {
    setLocked(false);
    scrollToLatest();
  }, [scrollToLatest]);
  const handleScroll = useCallback(() => {
    if (!programmaticScrollRef.current) lock();
  }, [lock]);

  useLayoutEffect(() => {
    if (!locked) scrollToLatest();
  });
  useEffect(
    () => () => {
      if (animationFrameRef.current !== undefined) {
        window.cancelAnimationFrame(animationFrameRef.current);
      }
    },
    [],
  );

  return { containerRef, handleScroll, lock, locked, unlock };
}

function ScrollLockButton({
  locked,
  onToggle,
  target,
}: {
  locked: boolean;
  onToggle: () => void;
  target: string;
}) {
  return (
    <Button
      className="scroll-lock-button"
      type="text"
      size="small"
      icon={locked ? <UnlockOutlined /> : <LockOutlined />}
      aria-pressed={locked}
      title={
        locked ? `点击解锁并自动滚动到最新${target}` : `点击锁定${target}滚动`
      }
      onClick={onToggle}
    >
      {locked ? "解锁" : "锁定"}
    </Button>
  );
}

function ThinkingStream({ text }: { text: string }) {
  const scroll = useAutoFollowScroll();
  return (
    <div className="thinking-stream">
      <div className="thinking-stream-header">
        <Typography.Text type="secondary">Thinking</Typography.Text>
        <ScrollLockButton
          locked={scroll.locked}
          target=" Thinking"
          onToggle={scroll.locked ? scroll.unlock : scroll.lock}
        />
      </div>
      <div
        ref={scroll.containerRef}
        className="thinking-stream-content"
        onScroll={scroll.handleScroll}
        onWheel={scroll.lock}
        onTouchMove={scroll.lock}
      >
        <Typography.Paragraph type="secondary">{text}</Typography.Paragraph>
      </div>
    </div>
  );
}

export function MessageList({
  messages,
  streamingText,
  thinkingText,
  workingAgentCode,
  starterPrompt,
  followLatestRequest,
  disabled,
  onStarter,
}: {
  messages: ConversationMessage[];
  streamingText?: string;
  thinkingText?: string;
  workingAgentCode?: string;
  starterPrompt?: string;
  followLatestRequest: number;
  disabled: boolean;
  onStarter: (content: string) => Promise<unknown>;
}) {
  const scroll = useAutoFollowScroll();
  useLayoutEffect(() => {
    if (followLatestRequest > 0) scroll.unlock();
  }, [followLatestRequest, scroll.unlock]);

  if (!messages.length) {
    return (
      <div className="chat-empty">
        {starterPrompt ? (
          <>
            <Typography.Text type="secondary">推荐从这里开始</Typography.Text>
            <Button
              className="starter-prompt"
              disabled={disabled}
              onClick={() => void onStarter(starterPrompt)}
            >
              {starterPrompt}
            </Button>
          </>
        ) : (
          <Typography.Text type="secondary">
            描述当前素材目标，Agent 会通过 Action 给出下一步。
          </Typography.Text>
        )}
      </div>
    );
  }
  return (
    <div className="message-list-shell">
      <div className="message-list-toolbar">
        <ScrollLockButton
          locked={scroll.locked}
          target="对话"
          onToggle={scroll.locked ? scroll.unlock : scroll.lock}
        />
      </div>
      <div
        ref={scroll.containerRef}
        className="message-list"
        onScroll={scroll.handleScroll}
        onWheel={scroll.lock}
        onTouchMove={scroll.lock}
      >
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
            {item.status === "thinking" &&
              thinkingText &&
              item.agentCode === workingAgentCode && (
                <ThinkingStream text={thinkingText} />
              )}
            {item.status === "thinking" && !item.content ? (
              streamingText && item.agentCode === workingAgentCode ? (
                <Typography.Paragraph>
                  {stripActionBlock(streamingText)}
                </Typography.Paragraph>
              ) : thinkingText && item.agentCode === workingAgentCode ? null : (
                <Spin size="small" />
              )
            ) : (
              <Typography.Paragraph>{item.content}</Typography.Paragraph>
            )}
          </article>
        ))}
      </div>
    </div>
  );
}

export function Composer({
  value,
  agents,
  busy,
  interrupting,
  disabled,
  onChange,
  onSend,
  onInterrupt,
}: {
  value: string;
  agents: AiAgent[];
  busy: boolean;
  interrupting?: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
  onSend: () => void;
  onInterrupt: () => void;
}) {
  return (
    <div className="chat-composer">
      <Mentions
        value={value}
        autoSize={{ minRows: 3, maxRows: 8 }}
        disabled={disabled || busy}
        placeholder="描述素材要求，输入 @ 可指定 Agent"
        options={agents.map((agent) => ({
          value: agent.role,
          label: `${agent.role} · ${agent.agentCode}`,
        }))}
        onChange={onChange}
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
        icon={busy ? <StopOutlined /> : <SendOutlined />}
        loading={busy && interrupting}
        disabled={disabled || interrupting || (!busy && !value.trim())}
        onClick={busy ? onInterrupt : onSend}
      >
        {busy ? "中断会话" : "发送"}
      </Button>
    </div>
  );
}

function findMentionedAgent(value: string, agents: AiAgent[]) {
  const agentByMention = new Map<string, string>();
  for (const agent of agents) {
    agentByMention.set(agent.agentCode.toLowerCase(), agent.agentCode);
    agentByMention.set(agent.role.toLowerCase(), agent.agentCode);
    for (const alias of agent.aliases) {
      agentByMention.set(alias.toLowerCase(), agent.agentCode);
    }
  }
  for (const match of value.matchAll(/@([^\s]+)/g)) {
    const agentCode = agentByMention.get(match[1].toLowerCase());
    if (agentCode) return agentCode;
  }
  return undefined;
}

function stripActionBlock(value: string) {
  return value.split("<-------- ACTION-START------->", 1)[0].trimEnd();
}
