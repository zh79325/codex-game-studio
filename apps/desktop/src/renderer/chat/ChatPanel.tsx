import {
  AudioOutlined,
  CloseOutlined,
  SendOutlined,
  StopOutlined,
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
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import type {
  AiAgent,
  ArtifactDraft,
  ConversationMessage,
  ConversationSnapshot,
} from "../types";
import InteractionDrawer from "./InteractionDrawer";
import { useRealtimeSpeech } from "./useRealtimeSpeech";

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
  const speech = useRealtimeSpeech({
    enabled: props.canWrite && !props.busy && Boolean(props.snapshot),
    onCompleted: (text) =>
      setContent((current) =>
        current && !/\s$/.test(current) ? `${current} ${text}` : `${current}${text}`,
      ),
    onError: (error) => message.error(error),
  });
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
          autoFollow={props.busy}
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
          voiceMode={speech.voiceMode}
          recording={speech.recording}
          speechWaiting={speech.waiting}
          speechTranscript={speech.transcript}
          onChange={setContent}
          onEnterVoice={speech.enterVoiceMode}
          onLeaveVoice={speech.leaveVoiceMode}
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

function useAutoFollowScroll(enabled: boolean) {
  const containerRef = useRef<HTMLDivElement>(null);
  const wasEnabledRef = useRef(enabled);

  const scrollToLatest = useCallback(() => {
    const container = containerRef.current;
    if (container) container.scrollTop = container.scrollHeight;
  }, []);

  useLayoutEffect(() => {
    if (enabled || wasEnabledRef.current) scrollToLatest();
    wasEnabledRef.current = enabled;
  });

  return { containerRef, scrollToLatest };
}

function ThinkingStream({ text }: { text: string }) {
  const scroll = useAutoFollowScroll(true);
  return (
    <div className="thinking-stream">
      <Typography.Text type="secondary">Thinking</Typography.Text>
      <div ref={scroll.containerRef} className="thinking-stream-content">
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
  autoFollow,
  followLatestRequest,
  disabled,
  onStarter,
}: {
  messages: ConversationMessage[];
  streamingText?: string;
  thinkingText?: string;
  workingAgentCode?: string;
  starterPrompt?: string;
  autoFollow: boolean;
  followLatestRequest: number;
  disabled: boolean;
  onStarter: (content: string) => Promise<unknown>;
}) {
  const scroll = useAutoFollowScroll(autoFollow);
  useLayoutEffect(() => {
    if (followLatestRequest > 0) scroll.scrollToLatest();
  }, [followLatestRequest, scroll.scrollToLatest]);

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
      <div ref={scroll.containerRef} className="message-list">
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
  voiceMode,
  recording,
  speechWaiting,
  speechTranscript,
  onChange,
  onEnterVoice,
  onLeaveVoice,
  onSend,
  onInterrupt,
}: {
  value: string;
  agents: AiAgent[];
  busy: boolean;
  interrupting?: boolean;
  disabled: boolean;
  voiceMode: boolean;
  recording: boolean;
  speechWaiting: boolean;
  speechTranscript: string;
  onChange: (value: string) => void;
  onEnterVoice: () => void;
  onLeaveVoice: () => void;
  onSend: () => void;
  onInterrupt: () => void;
}) {
  return (
    <div className="chat-composer">
      <div className="chat-composer-input">
        <Mentions
          value={value}
          autoSize={{ minRows: 3, maxRows: 8 }}
          disabled={disabled || busy}
          readOnly={voiceMode}
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
        {voiceMode && (
          <div
            className={`speech-input ${recording || speechWaiting ? "speech-input-active" : ""} ${recording ? "speech-input-recording" : ""}`}
            role="status"
            aria-live="polite"
          >
            <div className="speech-wave" aria-hidden="true">
              {Array.from({ length: 7 }, (_, index) => (
                <span key={index} />
              ))}
            </div>
            <Typography.Text>
              {speechTranscript ||
                (speechWaiting
                  ? "正在识别整句…"
                  : recording
                    ? "正在录音，松开空格键发送语音"
                    : "长按空格键开始进行语音输入")}
            </Typography.Text>
          </div>
        )}
      </div>
      <Space className="chat-composer-actions">
        <Button
          aria-label={voiceMode ? "退出语音输入" : "进入语音输入"}
          type={voiceMode ? "primary" : "default"}
          danger={recording}
          icon={voiceMode ? <CloseOutlined /> : <AudioOutlined />}
          disabled={disabled || busy}
          onClick={voiceMode ? onLeaveVoice : onEnterVoice}
        />
        <Button
          type={busy ? "default" : "primary"}
          danger={busy}
          icon={busy ? <StopOutlined /> : <SendOutlined />}
          loading={busy && interrupting}
          disabled={
            disabled ||
            interrupting ||
            voiceMode ||
            (!busy && !value.trim())
          }
          onClick={busy ? onInterrupt : onSend}
        >
          {busy ? "中断会话" : "发送"}
        </Button>
      </Space>
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
