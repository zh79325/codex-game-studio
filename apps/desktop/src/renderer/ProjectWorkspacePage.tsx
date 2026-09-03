import {
  CheckCircleOutlined,
  HistoryOutlined,
  SendOutlined,
} from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Alert,
  App,
  Button,
  Card,
  Col,
  Collapse,
  Empty,
  Input,
  List,
  Row,
  Space,
  Steps,
  Tag,
  Typography,
} from "antd";
import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import type { GameConflict } from "../generated/game";
import { rpc, workspaceApi } from "./api";
import { useStudio } from "./AppShell";

const focusSteps = [
  { title: "游戏简报", states: ["CLARIFYING", "BRIEF_READY"] },
  { title: "并行评审", states: ["REVIEWING", "MERGING"] },
  { title: "冲突决策", states: ["USER_REVIEW"] },
  { title: "Art Bible", states: ["CONFIRMED", "VERSIONED"] },
];

export default function ProjectWorkspacePage() {
  const { message } = App.useApp();
  const { projectId = "" } = useParams();
  const { canWrite, activeProject, setActiveProject } = useStudio();
  const queryClient = useQueryClient();
  const [content, setContent] = useState("");
  const [selectedVersion, setSelectedVersion] = useState<number>();

  const project = useQuery({
    queryKey: ["project", projectId],
    queryFn: () => workspaceApi.readProject(projectId),
    enabled: Boolean(projectId),
  });

  useEffect(() => {
    if (project.data && activeProject?.id !== project.data.id) {
      setActiveProject(project.data);
    }
  }, [activeProject?.id, project.data, setActiveProject]);

  const conversation = useQuery({
    queryKey: ["conversation", projectId],
    queryFn: async () => {
      const ensured = await workspaceApi.ensureConversation(projectId);
      await workspaceApi.startFocus(ensured.id);
      return ensured;
    },
    enabled: Boolean(projectId),
  });
  const conversationId = conversation.data?.id;

  const focus = useQuery({
    queryKey: ["focus", conversationId],
    queryFn: () => workspaceApi.readFocus(conversationId!),
    enabled: Boolean(conversationId),
  });
  const tasks = useQuery({
    queryKey: ["tasks", conversationId],
    queryFn: () => workspaceApi.listTasks(conversationId!),
    enabled: Boolean(conversationId),
  });
  const versions = useQuery({
    queryKey: ["art-bible-versions", projectId],
    queryFn: () => workspaceApi.listVersions(projectId),
    enabled: Boolean(projectId),
  });
  const artBible = useQuery({
    queryKey: ["art-bible", projectId, selectedVersion],
    queryFn: () =>
      rpc<{ markdown: string }>("game/artBible/read", {
        projectId,
        version: selectedVersion,
      }),
    enabled: selectedVersion !== undefined,
  });

  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["focus", conversationId] }),
      queryClient.invalidateQueries({ queryKey: ["tasks", conversationId] }),
      queryClient.invalidateQueries({ queryKey: ["art-bible-versions", projectId] }),
    ]);
  };

  const submit = useMutation({
    mutationFn: () =>
      rpc("game/conversation/submit", {
        conversationId,
        content: content.trim(),
      }),
    onSuccess: async () => {
      setContent("");
      await refresh();
    },
    onError: (error: Error) => message.error(error.message),
  });

  const decide = useMutation({
    mutationFn: ({ action, extras = {} }: { action: string; extras?: Record<string, unknown> }) =>
      rpc("game/focus/decide", {
        conversationId,
        expectedInputVersion: Number(focus.data?.workflow.inputVersion ?? 0),
        action,
        ...extras,
      }),
    onSuccess: refresh,
    onError: (error: Error) => message.error(error.message),
  });

  const currentStep = Math.max(
    0,
    focusSteps.findIndex((step) => step.states.includes(focus.data?.workflow.state ?? "")),
  );
  const unresolvedHighImpact =
    focus.data?.conflicts.some(
      (conflict) =>
        conflict.highImpact &&
        !focus.data?.decisions.some((decision) => decision.conflictKey === conflict.key),
    ) ?? false;

  return (
    <div className="page-stack workspace-page">
      <section className="page-heading">
        <div>
          <Space align="center">
            <Typography.Title level={2}>{project.data?.name ?? "项目工作区"}</Typography.Title>
            {project.data?.state && <Tag color={canWrite ? "blue" : "warning"}>{project.data.state}</Tag>}
          </Space>
          <Typography.Text type="secondary" className="path-text">
            {project.data?.root}
          </Typography.Text>
        </div>
      </section>

      {!canWrite && (
        <Alert type="warning" showIcon message="当前为只读模式，浏览功能可用，所有提交操作已禁用。" />
      )}

      <Row gutter={[16, 16]} align="stretch">
        <Col xs={24} xl={6}>
          <Card title="对焦流程" className="content-card full-height-card">
            <Steps direction="vertical" size="small" current={currentStep} items={focusSteps.map(({ title }) => ({ title }))} />
            <Typography.Title level={5}>Art Bible 版本</Typography.Title>
            <Space wrap>
              {(versions.data ?? []).map((version) => (
                <Button
                  key={version.id}
                  type={selectedVersion === Number(version.version) ? "primary" : "default"}
                  icon={<HistoryOutlined />}
                  onClick={() => setSelectedVersion(Number(version.version))}
                >
                  v{String(version.version)}
                </Button>
              ))}
              {!versions.data?.length && <Typography.Text type="secondary">暂无版本</Typography.Text>}
            </Space>
          </Card>
        </Col>

        <Col xs={24} xl={18}>
          <Space direction="vertical" size="middle" className="workspace-main">
            <Card
              title={focus.data ? `当前阶段：${focus.data.workflow.state}` : "开始设定对焦"}
              extra={focus.data && <Tag>输入版本 {String(focus.data.workflow.inputVersion)}</Tag>}
              loading={focus.isLoading || conversation.isLoading}
              className="content-card"
            >
              {!focus.data && !focus.isLoading && <Empty description="描述你的游戏创意，开始设定对焦" />}
              {focus.data?.workflow.state === "BRIEF_READY" && (
                <Button
                  type="primary"
                  disabled={!canWrite}
                  loading={decide.isPending}
                  onClick={() => decide.mutate({ action: "acceptBrief" })}
                >
                  接受简报并开始评审
                </Button>
              )}

              {Boolean(focus.data?.reviews.length) && (
                <section className="workspace-section">
                  <Typography.Title level={4}>评审报告</Typography.Title>
                  <Row gutter={[12, 12]}>
                    {focus.data?.reviews.map((review) => (
                      <Col xs={24} lg={12} key={review.agentCode}>
                        <Card size="small" title={review.agentCode}>
                          <ReviewList title="发现" items={review.findings} />
                          <ReviewList title="风险" items={review.risks} />
                          <ReviewList title="建议" items={review.recommendations} />
                        </Card>
                      </Col>
                    ))}
                  </Row>
                </section>
              )}

              {Boolean(focus.data?.conflicts.length) && (
                <section className="workspace-section">
                  <Typography.Title level={4}>冲突决策</Typography.Title>
                  <Space direction="vertical" className="workspace-main">
                    {focus.data?.conflicts.map((conflict) => (
                      <ConflictCard
                        key={conflict.key}
                        conflict={conflict}
                        selected={focus.data?.decisions.find((item) => item.conflictKey === conflict.key)?.selectedOption}
                        disabled={!canWrite || decide.isPending}
                        onSelect={(option) =>
                          decide.mutate({
                            action: "recordConflictDecision",
                            extras: {
                              userDecision: { conflictKey: conflict.key, selectedOption: option, note: null },
                            },
                          })
                        }
                      />
                    ))}
                  </Space>
                </section>
              )}

              {focus.data?.artBibleDraft && (
                <section className="workspace-section">
                  <Space className="section-heading">
                    <Typography.Title level={4}>Art Bible 草案</Typography.Title>
                    {focus.data.workflow.state === "USER_REVIEW" && (
                      <Button
                        type="primary"
                        icon={<CheckCircleOutlined />}
                        disabled={!canWrite || unresolvedHighImpact}
                        onClick={() =>
                          decide.mutate({
                            action: "confirmArtBible",
                            extras: { artBibleMarkdown: focus.data?.artBibleDraft },
                          })
                        }
                      >
                        确认 Art Bible
                      </Button>
                    )}
                    {focus.data.workflow.state === "CONFIRMED" && (
                      <Button disabled={!canWrite} onClick={() => decide.mutate({ action: "versionArtBible" })}>
                        完成版本化
                      </Button>
                    )}
                  </Space>
                  <pre className="document">{focus.data.artBibleDraft}</pre>
                </section>
              )}
            </Card>

            {artBible.data && (
              <Card title={`历史版本 v${selectedVersion}`} className="content-card">
                <pre className="document">{artBible.data.markdown}</pre>
              </Card>
            )}

            <Card title="任务" className="content-card" loading={tasks.isLoading}>
              <Space wrap>
                {(tasks.data ?? []).map((task) => (
                  <Tag key={task.id} color={task.status === "succeeded" ? "success" : task.status === "failed" ? "error" : "processing"}>
                    {task.agentCode} · {task.status}
                  </Tag>
                ))}
                {!tasks.data?.length && <Typography.Text type="secondary">暂无任务</Typography.Text>}
              </Space>
            </Card>

            <Card className="composer-card">
              <Input.TextArea
                autoSize={{ minRows: 3, maxRows: 8 }}
                value={content}
                disabled={!canWrite || !conversationId}
                placeholder="输入游戏创意或补充信息"
                onChange={(event) => setContent(event.target.value)}
              />
              <Button
                type="primary"
                icon={<SendOutlined />}
                loading={submit.isPending}
                disabled={!canWrite || !conversationId || !content.trim()}
                onClick={() => submit.mutate()}
              >
                提交
              </Button>
            </Card>

            <Collapse
              ghost
              items={[{ key: "debug", label: "运行状态", children: <pre>{JSON.stringify({ conversationId, focus: focus.data?.workflow }, null, 2)}</pre> }]}
            />
          </Space>
        </Col>
      </Row>
    </div>
  );
}

function ReviewList({ title, items }: { title: string; items: string[] }) {
  return (
    <div className="review-list">
      <Typography.Text strong>{title}</Typography.Text>
      <List size="small" dataSource={items} renderItem={(item) => <List.Item>{item}</List.Item>} />
    </div>
  );
}

function ConflictCard({
  conflict,
  selected,
  disabled,
  onSelect,
}: {
  conflict: GameConflict;
  selected?: string;
  disabled: boolean;
  onSelect: (option: string) => void;
}) {
  return (
    <Card
      size="small"
      title={conflict.description}
      extra={conflict.highImpact && <Tag color="warning">高影响</Tag>}
    >
      <Space wrap>
        {conflict.options.map((option) => (
          <Button
            key={option}
            type={selected === option ? "primary" : "default"}
            disabled={disabled}
            onClick={() => onSelect(option)}
          >
            {option}
          </Button>
        ))}
      </Space>
    </Card>
  );
}
