import { CheckCircleOutlined, PlusOutlined } from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Alert,
  App,
  Button,
  Card,
  Col,
  Form,
  Input,
  List,
  Modal,
  Row,
  Space,
  Steps,
  Tag,
  Typography,
} from "antd";
import { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { aiApi, charactersApi, workspaceApi } from "./api";
import { useStudio } from "./AppShell";
import ChatPanel from "./chat/ChatPanel";
import { useConversation } from "./chat/useConversation";
import type { ArtifactDraft } from "./types";

const projectSteps = [
  { title: "定义美术基调", state: "drafting" },
  { title: "确认游戏风格", state: "styleSettled" },
  { title: "确认项目名称与代号", state: "ready" },
];

export default function ProjectWorkspacePage() {
  const { message } = App.useApp();
  const { projectId = "" } = useParams();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { canWrite, activeProject, setActiveProject } = useStudio();
  const [finalizeOpen, setFinalizeOpen] = useState(false);
  const [characterOpen, setCharacterOpen] = useState(false);
  const [finalizeForm] = Form.useForm<{ name: string; code: string }>();
  const [characterForm] = Form.useForm<{ name: string; group?: string }>();

  const project = useQuery({
    queryKey: ["project", projectId],
    queryFn: () => workspaceApi.readProject(projectId),
    enabled: Boolean(projectId),
  });
  const agents = useQuery({
    queryKey: ["ai-agents"],
    queryFn: aiApi.listAgents,
  });
  const characters = useQuery({
    queryKey: ["characters", projectId],
    queryFn: () => charactersApi.list(projectId),
    enabled: project.data?.state === "ready",
  });
  const conversation = useConversation({
    projectId,
    targetKind: "project",
    targetRef: null,
    title: "项目美术基调",
  });

  useEffect(() => {
    if (project.data && activeProject?.id !== project.data.id)
      setActiveProject(project.data);
  }, [activeProject?.id, project.data, setActiveProject]);

  const latestNaming = useMemo(
    () =>
      conversation.snapshot?.messages
        .slice()
        .reverse()
        .find((item) => item.action?.payload.naming?.length)?.action?.payload
        .naming ?? [],
    [conversation.snapshot?.messages],
  );

  const refreshProject = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["project", projectId] }),
      queryClient.invalidateQueries({ queryKey: ["projects"] }),
      queryClient.invalidateQueries({ queryKey: ["characters", projectId] }),
      conversation.refresh(),
    ]);
  };

  const commitArtBible = useMutation({
    mutationFn: (draftId: string) =>
      workspaceApi.commitArtBible(conversation.conversationId!, draftId),
    onSuccess: async () => {
      message.success("游戏风格已确认");
      await refreshProject();
      await conversation.send(
        "游戏风格已由用户确认。请基于已确认的 Art Bible 给出 2–3 组项目名称和合法项目代号建议，并在 Action payload.naming 中返回。",
        "game_designer",
      );
    },
    onError: (error: Error) => message.error(error.message),
  });

  const finalize = useMutation({
    mutationFn: ({ name, code }: { name: string; code: string }) =>
      workspaceApi.finalize(projectId, name, code),
    onSuccess: async (updated) => {
      setActiveProject(updated);
      setFinalizeOpen(false);
      message.success("项目立项完成");
      await refreshProject();
    },
    onError: (error: Error) => message.error(error.message),
  });

  const createCharacter = useMutation({
    mutationFn: ({ name, group }: { name: string; group?: string }) =>
      charactersApi.create(projectId, name, group?.trim() || null, false),
    onSuccess: async (character) => {
      setCharacterOpen(false);
      characterForm.resetFields();
      await queryClient.invalidateQueries({
        queryKey: ["characters", projectId],
      });
      navigate(`/projects/${projectId}/characters/${character.id}`);
    },
    onError: (error: Error) => message.error(error.message),
  });

  const currentStep =
    project.data?.state === "ready"
      ? 2
      : project.data?.state === "styleSettled"
        ? 1
        : 0;
  const renderDraftAction = (draft: ArtifactDraft) =>
    draft.targetPath === "art-bible.md" &&
    project.data?.state === "drafting" ? (
      <Button
        type="primary"
        size="small"
        icon={<CheckCircleOutlined />}
        disabled={!canWrite}
        loading={commitArtBible.isPending}
        onClick={() => commitArtBible.mutate(draft.id)}
      >
        确认游戏风格
      </Button>
    ) : null;

  return (
    <div className="page-stack workspace-page">
      {!canWrite && (
        <Alert
          type="warning"
          showIcon
          message="当前为只读模式，所有提交操作已禁用。"
        />
      )}
      <Row gutter={[16, 16]} align="stretch">
        <Col xs={24} xl={7}>
          <Space direction="vertical" className="workspace-main">
            <Card
              title="立项流程"
              className="content-card"
              extra={
                project.data?.state === "styleSettled" ? (
                  <Button
                    type="primary"
                    size="small"
                    disabled={!canWrite}
                    onClick={() => setFinalizeOpen(true)}
                  >
                    确认立项
                  </Button>
                ) : null
              }
            >
              <Space direction="vertical" className="workspace-main">
                <div>
                  <Space align="center" wrap>
                    <Typography.Text strong>
                      {project.data?.name ?? "素材项目立项"}
                    </Typography.Text>
                    {project.data?.state && (
                      <Tag
                        color={
                          project.data.state === "ready"
                            ? "success"
                            : "processing"
                        }
                      >
                        {project.data.state}
                      </Tag>
                    )}
                  </Space>
                  <Typography.Text type="secondary" className="path-text">
                    {project.data?.root}
                  </Typography.Text>
                </div>
                <Steps
                  direction="vertical"
                  current={currentStep}
                  items={projectSteps}
                />
              </Space>
            </Card>
            <Card
              title="角色"
              className="content-card"
              extra={
                <Button
                  size="small"
                  icon={<PlusOutlined />}
                  disabled={!canWrite || project.data?.state !== "ready"}
                  onClick={() => setCharacterOpen(true)}
                >
                  新建
                </Button>
              }
            >
              {project.data?.state !== "ready" ? (
                <Typography.Text type="secondary">
                  确认 Art Bible 和项目名称后才能创建角色。
                </Typography.Text>
              ) : (
                <List
                  dataSource={characters.data ?? []}
                  locale={{ emptyText: "还没有角色" }}
                  renderItem={(character) => (
                    <List.Item
                      actions={[
                        <Button
                          key="open"
                          type="link"
                          onClick={() =>
                            navigate(
                              `/projects/${projectId}/characters/${character.id}`,
                            )
                          }
                        >
                          进入
                        </Button>,
                      ]}
                    >
                      <List.Item.Meta
                        title={character.name}
                        description={`${character.group ?? "未分组"} · ${character.state}`}
                      />
                    </List.Item>
                  )}
                />
              )}
            </Card>
          </Space>
        </Col>
        <Col xs={24} xl={17}>
          <ChatPanel
            snapshot={conversation.snapshot}
            agents={agents.data}
            loading={conversation.isLoading}
            canWrite={canWrite}
            streamingText={conversation.streamingText}
            workingAgentCode={conversation.workingAgentCode}
            lastError={conversation.lastError}
            starterPrompt="我要开发一款类似我的世界地下城的刷怪RPG，玩家扮演的角色是西游记中的人物例如孙悟空，猪八戒，二郎神等，怪物是类似奥特曼电视剧中的怪兽，场景是在现代各个城市的地标建筑附近。"
            onSend={conversation.send}
            onInterrupt={conversation.interrupt}
            onCommitDrafts={conversation.commitDrafts}
            renderDraftAction={renderDraftAction}
          />
        </Col>
      </Row>

      <Modal
        title="确认项目立项"
        open={finalizeOpen}
        okText="确认立项"
        confirmLoading={finalize.isPending}
        onCancel={() => setFinalizeOpen(false)}
        onOk={() => finalizeForm.submit()}
      >
        <Form
          form={finalizeForm}
          layout="vertical"
          onFinish={(values) => finalize.mutate(values)}
        >
          <Form.Item
            name="name"
            label="项目名称"
            rules={[{ required: true, whitespace: true }]}
          >
            <Input placeholder="项目名称" />
          </Form.Item>
          <Form.Item
            name="code"
            label="项目代号"
            rules={[
              { required: true },
              {
                pattern: /^[a-z0-9][a-z0-9_-]*$/,
                message: "仅允许小写字母、数字、_、-，并以字母或数字开头",
              },
            ]}
          >
            <Input placeholder="project-code" />
          </Form.Item>
          {latestNaming.length > 0 && (
            <Space direction="vertical" className="workspace-main">
              <Typography.Text type="secondary">Agent 建议</Typography.Text>
              {latestNaming.map((suggestion) => (
                <Button
                  key={`${suggestion.name}-${suggestion.code}`}
                  onClick={() =>
                    finalizeForm.setFieldsValue({
                      name: suggestion.name,
                      code: suggestion.code,
                    })
                  }
                >
                  {suggestion.name} · {suggestion.code}
                </Button>
              ))}
            </Space>
          )}
        </Form>
      </Modal>

      <Modal
        title="新建角色"
        open={characterOpen}
        okText="创建并进入"
        confirmLoading={createCharacter.isPending}
        onCancel={() => setCharacterOpen(false)}
        onOk={() => characterForm.submit()}
      >
        <Form
          form={characterForm}
          layout="vertical"
          onFinish={(values) => createCharacter.mutate(values)}
        >
          <Form.Item
            name="name"
            label="角色名称"
            rules={[{ required: true, whitespace: true }]}
          >
            <Input autoFocus />
          </Form.Item>
          <Form.Item name="group" label="分组（可选）">
            <Input />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
