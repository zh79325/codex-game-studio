import {
  CheckCircleOutlined,
  DeleteOutlined,
  FolderAddOutlined,
  FolderOutlined,
  UserAddOutlined,
} from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Alert,
  App,
  Button,
  Card,
  Col,
  Form,
  Input,
  Modal,
  Row,
  Select,
  Space,
  Steps,
  Tabs,
  Tag,
  Tree,
  Typography,
} from "antd";
import { useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { aiApi, charactersApi, workspaceApi } from "./api";
import { useStudio } from "./AppShell";
import ChatPanel from "./chat/ChatPanel";
import type { ChoiceSubmission } from "./chat/ChoiceQuestions";
import { useConversation } from "./chat/useConversation";
import type { ArtifactDraft, ChoiceGroup, ListedCharacter } from "./types";

const projectSteps = [
  { title: "定义美术基调", state: "drafting" },
  { title: "确认游戏风格", state: "styleSettled" },
  { title: "确认项目名称与代号", state: "ready" },
];

export default function ProjectWorkspacePage() {
  const { message, modal } = App.useApp();
  const { projectId = "" } = useParams();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { canWrite, activeProject, setActiveProject } = useStudio();
  const [characterOpen, setCharacterOpen] = useState(false);
  const [groupOpen, setGroupOpen] = useState(false);
  const [characterForm] = Form.useForm<{ name: string; group?: string }>();
  const [groupForm] = Form.useForm<{ name: string }>();

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
        "游戏风格已由用户确认。请基于已确认的 Art Bible 给出 2–3 组项目名称和合法项目代号建议，并在 Action payload.choices 中返回两组单选题：项目名称、项目代号。",
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
      message.success("项目立项完成");
      await refreshProject();
    },
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
  const createCharacterGroup = useMutation({
    mutationFn: ({ name }: { name: string }) =>
      charactersApi.createGroup(projectId, name.trim()),
    onSuccess: async (group) => {
      setGroupOpen(false);
      groupForm.resetFields();
      characterForm.setFieldValue("group", group);
      await queryClient.invalidateQueries({
        queryKey: ["characters", projectId],
      });
      message.success(`已新建分组“${group}”`);
    },
    onError: (error: Error) => message.error(error.message),
  });
  const removeCharacter = useMutation({
    mutationFn: (characterId: string) =>
      charactersApi.remove(projectId, characterId),
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["characters", projectId],
      });
      message.success("已移除失效角色记录");
    },
    onError: (error: Error) => message.error(error.message),
  });

  const hasPendingArtBible = conversation.snapshot?.drafts.some(
    (draft) =>
      draft.targetPath === "art-bible.md" && draft.status === "pending",
  );
  const currentStep =
    project.data?.state === "ready"
      ? 3
      : project.data?.state === "styleSettled"
        ? 2
        : hasPendingArtBible
          ? 1
          : 0;
  const resolveProjectNaming = async (
    groups: ChoiceGroup[],
    submission: ChoiceSubmission,
  ) => {
    if (
      project.data?.state !== "styleSettled" ||
      !isProjectNamingChoice(groups)
    ) {
      return false;
    }
    const name = submission.answers.find((answer) => answer.item === "项目名称")
      ?.values[0];
    const code = submission.answers.find((answer) => answer.item === "项目代号")
      ?.values[0];
    if (!name || !code) throw new Error("请选择项目名称和项目代号");
    await finalize.mutateAsync({ name, code });
    return true;
  };
  const renderDraftAction = (draft: ArtifactDraft, closeDrawer: () => void) =>
    draft.targetPath === "art-bible.md" &&
    project.data?.state === "drafting" ? (
      <Button
        type="primary"
        size="small"
        icon={<CheckCircleOutlined />}
        disabled={!canWrite}
        loading={commitArtBible.isPending}
        onClick={() =>
          commitArtBible.mutate(draft.id, { onSuccess: closeDrawer })
        }
      >
        确认游戏风格
      </Button>
    ) : null;
  const characterList = characters.data?.characters ?? [];
  const characterGroups = characters.data?.groups ?? [];
  const renderCharacterNode = (character: ListedCharacter) => (
    <div className="character-tree-node">
      <div className="entity-list-content">
        <Typography.Text>{character.name}</Typography.Text>
        {character.modelFileExists ? (
          <Typography.Text type="secondary">{character.state}</Typography.Text>
        ) : (
          <Tag color="error">角色文件不存在</Tag>
        )}
      </div>
      {character.modelFileExists ? (
        <Button
          type="link"
          size="small"
          onClick={() =>
            navigate(`/projects/${projectId}/characters/${character.id}`)
          }
        >
          进入
        </Button>
      ) : (
        <Button
          danger
          type="link"
          size="small"
          icon={<DeleteOutlined />}
          disabled={!canWrite}
          loading={
            removeCharacter.isPending &&
            removeCharacter.variables === character.id
          }
          onClick={() =>
            modal.confirm({
              title: `移除失效角色“${character.name}”？`,
              content:
                "只会移除数据库中的失效角色记录，不会删除磁盘目录或其他文件。",
              okText: "移除",
              okButtonProps: { danger: true },
              cancelText: "取消",
              onOk: () => removeCharacter.mutateAsync(character.id),
            })
          }
        >
          移除
        </Button>
      )}
    </div>
  );
  const ungroupedCharacters = characterList.filter(
    (character) => !character.group,
  );
  const characterTree = [
    ...characterGroups.map((group) => {
      const groupedCharacters = characterList.filter(
        (character) => character.group === group,
      );
      return {
        key: `group:${group}`,
        icon: <FolderOutlined />,
        title: (
          <Space size="small">
            <Typography.Text strong>{group}</Typography.Text>
            <Typography.Text type="secondary">
              {groupedCharacters.length}
            </Typography.Text>
          </Space>
        ),
        children: groupedCharacters.map((character) => ({
          key: `character:${character.id}`,
          title: renderCharacterNode(character),
        })),
      };
    }),
    ...(ungroupedCharacters.length
      ? [
          {
            key: "group:ungrouped",
            icon: <FolderOutlined />,
            title: (
              <Space size="small">
                <Typography.Text strong>未分组</Typography.Text>
                <Typography.Text type="secondary">
                  {ungroupedCharacters.length}
                </Typography.Text>
              </Space>
            ),
            children: ungroupedCharacters.map((character) => ({
              key: `character:${character.id}`,
              title: renderCharacterNode(character),
            })),
          },
        ]
      : []),
  ];

  return (
    <div className="page-stack workspace-page">
      {!canWrite && (
        <Alert
          type="warning"
          showIcon
          title="当前为只读模式，所有提交操作已禁用。"
        />
      )}
      <Row gutter={[16, 16]} align="stretch">
        <Col xs={24} xl={7}>
          <Space orientation="vertical" className="workspace-main">
            <Card title="立项流程" className="content-card">
              <Space orientation="vertical" className="workspace-main">
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
                  orientation="vertical"
                  current={currentStep}
                  items={projectSteps}
                />
              </Space>
            </Card>
            <Card className="content-card asset-tabs-card">
              <Tabs
                defaultActiveKey="characters"
                items={[
                  {
                    key: "characters",
                    label: "角色",
                    children: (
                      <Space orientation="vertical" className="workspace-main">
                        <Space wrap>
                          <Button
                            size="small"
                            type="primary"
                            icon={<UserAddOutlined />}
                            disabled={
                              !canWrite || project.data?.state !== "ready"
                            }
                            onClick={() => setCharacterOpen(true)}
                          >
                            新建角色
                          </Button>
                          <Button
                            size="small"
                            icon={<FolderAddOutlined />}
                            disabled={
                              !canWrite || project.data?.state !== "ready"
                            }
                            onClick={() => setGroupOpen(true)}
                          >
                            新建分组
                          </Button>
                        </Space>
                        {project.data?.state !== "ready" ? (
                          <Typography.Text type="secondary">
                            确认 Art Bible 和项目名称后才能创建角色。
                          </Typography.Text>
                        ) : characterTree.length ? (
                          <Tree
                            className="character-tree"
                            blockNode
                            defaultExpandAll
                            selectable={false}
                            showIcon
                            treeData={characterTree}
                          />
                        ) : (
                          <Typography.Text type="secondary">
                            还没有角色或分组
                          </Typography.Text>
                        )}
                      </Space>
                    ),
                  },
                  {
                    key: "maps",
                    label: "地图",
                    children: (
                      <Typography.Text type="secondary">
                        地图素材将在这里管理。
                      </Typography.Text>
                    ),
                  },
                  {
                    key: "equipment",
                    label: "武器装备",
                    children: (
                      <Typography.Text type="secondary">
                        武器装备素材将在这里管理。
                      </Typography.Text>
                    ),
                  },
                ]}
              />
            </Card>
          </Space>
        </Col>
        <Col className="chat-column" xs={24} xl={17}>
          <ChatPanel
            snapshot={conversation.snapshot}
            agents={agents.data}
            loading={conversation.isLoading}
            canWrite={canWrite}
            busy={conversation.isBusy}
            interrupting={conversation.isInterrupting}
            streamingText={conversation.streamingText}
            thinkingText={conversation.thinkingText}
            workingAgentCode={conversation.workingAgentCode}
            lastError={conversation.lastError}
            starterPrompt="我要开发一款类似我的世界地下城的刷怪RPG，玩家扮演的角色是西游记中的人物例如孙悟空，猪八戒，二郎神等，怪物是类似奥特曼电视剧中的怪兽，场景是在现代各个城市的地标建筑附近。"
            onSend={conversation.send}
            onInterrupt={conversation.interrupt}
            onCommitDrafts={conversation.commitDrafts}
            choiceInteractionEnabled={project.data?.state !== "ready"}
            onResolveChoice={resolveProjectNaming}
            renderDraftAction={renderDraftAction}
          />
        </Col>
      </Row>

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
            <Select
              allowClear
              placeholder="未分组"
              options={characterGroups.map((group) => ({
                label: group,
                value: group,
              }))}
            />
          </Form.Item>
        </Form>
      </Modal>
      <Modal
        title="新建角色分组"
        open={groupOpen}
        okText="新建分组"
        confirmLoading={createCharacterGroup.isPending}
        onCancel={() => {
          setGroupOpen(false);
          groupForm.resetFields();
        }}
        onOk={() => groupForm.submit()}
      >
        <Form
          form={groupForm}
          layout="vertical"
          onFinish={(values) => createCharacterGroup.mutate(values)}
        >
          <Form.Item
            name="name"
            label="分组名称"
            rules={[{ required: true, whitespace: true }]}
          >
            <Input autoFocus placeholder="例如：主角、怪物、NPC" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}

function isProjectNamingChoice(groups: ChoiceGroup[]) {
  return (
    groups.length === 2 &&
    groups.every((group) => !group.multiple) &&
    groups.some((group) => group.item === "项目名称") &&
    groups.some((group) => group.item === "项目代号")
  );
}
