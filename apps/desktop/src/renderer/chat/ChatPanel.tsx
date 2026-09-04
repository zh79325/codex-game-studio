import { SendOutlined, StopOutlined } from "@ant-design/icons";
import {
  Alert,
  App,
  Button,
  Card,
  Checkbox,
  Input,
  Mentions,
  Radio,
  Space,
  Spin,
  Tag,
  Typography,
} from "antd";
import { useMemo, useState } from "react";
import type {
  AiAgent,
  ArtifactDraft,
  ChoiceGroup,
  ConversationMessage,
  ConversationSnapshot,
} from "../types";

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
  renderDraftAction?: (draft: ArtifactDraft) => React.ReactNode;
};

export default function ChatPanel(props: ChatPanelProps) {
  const { message } = App.useApp();
  const [content, setContent] = useState("");
  const [selectedDrafts, setSelectedDrafts] = useState<string[]>([]);
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
      await props.onSend(
        normalized,
        findMentionedAgent(normalized, availableAgents),
      );
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
        onChoice={send}
        disabled={!props.canWrite || props.busy || !props.snapshot}
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
          disabled={!props.canWrite || props.busy}
        />
      )}

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
  );
}

export function MessageList({
  messages,
  streamingText,
  thinkingText,
  workingAgentCode,
  starterPrompt,
  disabled,
  onChoice,
}: {
  messages: ConversationMessage[];
  streamingText?: string;
  thinkingText?: string;
  workingAgentCode?: string;
  starterPrompt?: string;
  disabled: boolean;
  onChoice: (content: string) => Promise<void>;
}) {
  if (!messages.length) {
    return (
      <div className="chat-empty">
        {starterPrompt ? (
          <>
            <Typography.Text type="secondary">推荐从这里开始</Typography.Text>
            <Button
              className="starter-prompt"
              disabled={disabled}
              onClick={() => void onChoice(starterPrompt)}
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
          {item.status === "thinking" &&
            thinkingText &&
            item.agentCode === workingAgentCode && (
              <div className="thinking-stream">
                <Typography.Text type="secondary">Thinking</Typography.Text>
                <Typography.Paragraph type="secondary">
                  {thinkingText}
                </Typography.Paragraph>
              </div>
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
              {item.action.payload.choices && (
                <ChoiceQuestions
                  groups={item.action.payload.choices}
                  disabled={disabled}
                  onSubmit={onChoice}
                />
              )}
            </div>
          )}
        </article>
      ))}
    </div>
  );
}

type ChoiceAnswer = {
  selectedOptions: string[];
  details: Record<string, string>;
  customSelected: boolean;
  customOption: string;
  customDetail: string;
};

const CUSTOM_CHOICE_VALUE = "__custom_choice__";

function ChoiceQuestions({
  groups,
  disabled,
  onSubmit,
}: {
  groups: ChoiceGroup[];
  disabled: boolean;
  onSubmit: (content: string) => Promise<void>;
}) {
  const [answers, setAnswers] = useState<ChoiceAnswer[]>(() =>
    groups.map(() => ({
      selectedOptions: [],
      details: {},
      customSelected: false,
      customOption: "",
      customDetail: "",
    })),
  );

  const updateAnswer = (
    groupIndex: number,
    update: (answer: ChoiceAnswer) => ChoiceAnswer,
  ) => {
    setAnswers((current) =>
      current.map((answer, index) =>
        index === groupIndex ? update(answer) : answer,
      ),
    );
  };

  const selectOption = (
    groupIndex: number,
    group: ChoiceGroup,
    option: string,
    selected: boolean,
  ) => {
    updateAnswer(groupIndex, (answer) => ({
      ...answer,
      selectedOptions: group.multiple
        ? selected
          ? [...new Set([...answer.selectedOptions, option])]
          : answer.selectedOptions.filter((value) => value !== option)
        : selected
          ? [option]
          : [],
      customSelected: group.multiple ? answer.customSelected : false,
    }));
  };

  const selectCustom = (
    groupIndex: number,
    group: ChoiceGroup,
    selected: boolean,
  ) => {
    updateAnswer(groupIndex, (answer) => ({
      ...answer,
      selectedOptions: group.multiple ? answer.selectedOptions : [],
      customSelected: selected,
    }));
  };

  const complete = groups.every((_, index) => {
    const answer = answers[index];
    return (
      answer.selectedOptions.length > 0 ||
      (answer.customSelected && Boolean(answer.customOption.trim()))
    );
  });

  return (
    <div className="choice-questions">
      {groups.map((group, groupIndex) => {
        const answer = answers[groupIndex];
        const singleValue = answer.customSelected
          ? CUSTOM_CHOICE_VALUE
          : answer.selectedOptions[0];
        return (
          <fieldset className="choice-group" key={`${groupIndex}-${group.item}`}>
            <legend>
              <Typography.Text strong>{group.item}</Typography.Text>
              <Tag>{group.multiple ? "多选" : "单选"}</Tag>
            </legend>
            <div className="choice-options">
              {group.multiple ? (
                group.options.map((option) => (
                  <ChoiceOptionRow
                    key={option}
                    control={
                      <Checkbox
                        checked={answer.selectedOptions.includes(option)}
                        disabled={disabled}
                        onChange={(event) =>
                          selectOption(
                            groupIndex,
                            group,
                            option,
                            event.target.checked,
                          )
                        }
                      >
                        {option}
                        {group.recommended.includes(option) ? "（推荐）" : ""}
                      </Checkbox>
                    }
                    detail={answer.details[option] ?? ""}
                    disabled={disabled}
                    onDetailChange={(detail) =>
                      updateAnswer(groupIndex, (current) => ({
                        ...current,
                        selectedOptions: [
                          ...new Set([...current.selectedOptions, option]),
                        ],
                        details: { ...current.details, [option]: detail },
                      }))
                    }
                  />
                ))
              ) : (
                <Radio.Group
                  className="choice-radio-group"
                  value={singleValue}
                  onChange={(event) => {
                    const value = event.target.value as string;
                    if (value === CUSTOM_CHOICE_VALUE) {
                      selectCustom(groupIndex, group, true);
                    } else {
                      selectOption(groupIndex, group, value, true);
                    }
                  }}
                >
                  {group.options.map((option) => (
                    <ChoiceOptionRow
                      key={option}
                      control={
                        <Radio value={option} disabled={disabled}>
                          {option}
                          {group.recommended.includes(option) ? "（推荐）" : ""}
                        </Radio>
                      }
                      detail={answer.details[option] ?? ""}
                      disabled={disabled}
                      onDetailChange={(detail) =>
                        updateAnswer(groupIndex, (current) => ({
                          ...current,
                          selectedOptions: [option],
                          customSelected: false,
                          details: { ...current.details, [option]: detail },
                        }))
                      }
                    />
                  ))}
                  <CustomChoiceRow
                    control={
                      <Radio value={CUSTOM_CHOICE_VALUE} disabled={disabled}>
                        其他（自定义）
                      </Radio>
                    }
                    answer={answer}
                    disabled={disabled}
                    onSelect={() => selectCustom(groupIndex, group, true)}
                    onChange={(change) =>
                      updateAnswer(groupIndex, (current) => ({
                        ...current,
                        selectedOptions: [],
                        customSelected: true,
                        ...change,
                      }))
                    }
                  />
                </Radio.Group>
              )}
              {group.multiple && (
                <CustomChoiceRow
                  control={
                    <Checkbox
                      checked={answer.customSelected}
                      disabled={disabled}
                      onChange={(event) =>
                        selectCustom(groupIndex, group, event.target.checked)
                      }
                    >
                      其他（自定义）
                    </Checkbox>
                  }
                  answer={answer}
                  disabled={disabled}
                  onSelect={() => selectCustom(groupIndex, group, true)}
                  onChange={(change) =>
                    updateAnswer(groupIndex, (current) => ({
                      ...current,
                      customSelected: true,
                      ...change,
                    }))
                  }
                />
              )}
            </div>
          </fieldset>
        );
      })}
      <Button
        type="primary"
        disabled={disabled || !complete}
        onClick={() => void onSubmit(formatChoiceAnswers(groups, answers))}
      >
        提交选择
      </Button>
    </div>
  );
}

function ChoiceOptionRow({
  control,
  detail,
  disabled,
  onDetailChange,
}: {
  control: React.ReactNode;
  detail: string;
  disabled: boolean;
  onDetailChange: (detail: string) => void;
}) {
  return (
    <div className="choice-option-row">
      <div className="choice-option-control">{control}</div>
      <Input
        value={detail}
        disabled={disabled}
        placeholder="补充说明（可选）"
        onChange={(event) => onDetailChange(event.target.value)}
      />
    </div>
  );
}

function CustomChoiceRow({
  control,
  answer,
  disabled,
  onSelect,
  onChange,
}: {
  control: React.ReactNode;
  answer: ChoiceAnswer;
  disabled: boolean;
  onSelect: () => void;
  onChange: (change: Partial<ChoiceAnswer>) => void;
}) {
  return (
    <div className="choice-option-row choice-custom-option">
      <div className="choice-option-control">{control}</div>
      <Input
        value={answer.customOption}
        disabled={disabled}
        placeholder="输入自定义选项"
        onFocus={onSelect}
        onChange={(event) => onChange({ customOption: event.target.value })}
      />
      <Input
        value={answer.customDetail}
        disabled={disabled}
        placeholder="补充说明（可选）"
        onFocus={onSelect}
        onChange={(event) => onChange({ customDetail: event.target.value })}
      />
    </div>
  );
}

function formatChoiceAnswers(groups: ChoiceGroup[], answers: ChoiceAnswer[]) {
  const lines = ["我的选择："];
  groups.forEach((group, index) => {
    const answer = answers[index];
    lines.push(`${index + 1}. ${group.item}`);
    for (const option of answer.selectedOptions) {
      const detail = answer.details[option]?.trim();
      lines.push(`- ${option}${detail ? `；补充：${detail}` : ""}`);
    }
    if (answer.customSelected) {
      const detail = answer.customDetail.trim();
      lines.push(
        `- ${answer.customOption.trim()}${detail ? `；补充：${detail}` : ""}`,
      );
    }
  });
  return lines.join("\n");
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

function isDedicatedGateDraft(draft: ArtifactDraft) {
  return (
    draft.targetPath === "art-bible.md" ||
    draft.targetPath === "docs/角色定稿.md"
  );
}
