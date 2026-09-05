import { CheckOutlined, ReloadOutlined } from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Card,
  Checkbox,
  Col,
  Input,
  Modal,
  Row,
  Space,
  Steps,
  Tag,
  Typography,
} from "antd";
import { useEffect, useRef, useState } from "react";
import { useParams } from "react-router-dom";
import { aiApi, charactersApi, workspaceApi } from "./api";
import { useStudio } from "./AppShell";
import ChatPanel from "./chat/ChatPanel";
import { useConversation } from "./chat/useConversation";
import type { ArtifactDraft, Generation } from "./types";

const states = [
  "S0_spec_drafting",
  "S1_spec_confirmed",
  "S2_render_generated",
  "S3_render_confirmed",
  "S4_views_generated",
  "S5_views_confirmed",
] as const;

const labels = [
  "设定草拟",
  "设定确认",
  "效果图候选",
  "效果图定稿",
  "四视图候选",
  "四视图定稿",
];

export default function CharacterPage() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const { projectId = "", characterId = "" } = useParams();
  const { canWrite, setActiveProject } = useStudio();
  const [selectedViews, setSelectedViews] = useState<string[]>([]);
  const [rejectOpen, setRejectOpen] = useState(false);
  const [rejectReason, setRejectReason] = useState("");
  const [requestingRedraw, setRequestingRedraw] = useState(false);
  const draftDrawerCloseRef = useRef<(() => void) | null>(null);

  const project = useQuery({
    queryKey: ["project", projectId],
    queryFn: () => workspaceApi.readProject(projectId),
    enabled: Boolean(projectId),
  });
  const detail = useQuery({
    queryKey: ["character", projectId, characterId],
    queryFn: () => charactersApi.read(projectId, characterId),
    enabled: Boolean(projectId && characterId),
  });
  const agents = useQuery({
    queryKey: ["ai-agents"],
    queryFn: aiApi.listAgents,
  });
  const conversation = useConversation(
    {
      projectId,
      targetKind: "character",
      targetRef: characterId,
      title: detail.data?.character.name ?? "角色素材",
    },
    Boolean(detail.data),
  );

  useEffect(() => {
    if (project.data) setActiveProject(project.data);
  }, [project.data, setActiveProject]);

  useEffect(
    () =>
      window.codexGame.onEvent((event) => {
        if (typeof event !== "object" || !event || !("method" in event)) return;
        const method = String(event.method);
        if (
          method === "game/character/updated" ||
          method === "game/generation/updated"
        ) {
          void queryClient.invalidateQueries({
            queryKey: ["character", projectId, characterId],
          });
          void queryClient.invalidateQueries({
            queryKey: ["characters", projectId],
          });
        }
      }),
    [characterId, projectId, queryClient],
  );

  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({
        queryKey: ["character", projectId, characterId],
      }),
      queryClient.invalidateQueries({ queryKey: ["characters", projectId] }),
      conversation.refresh(),
    ]);
  };
  const action = useMutation({
    mutationFn: async (
      operation:
        | { type: "spec"; draftId: string }
        | { type: "render"; generationId: string }
        | { type: "views"; generationIds: string[] },
    ) => {
      if (operation.type === "spec")
        return charactersApi.confirmSpec(
          projectId,
          characterId,
          operation.draftId,
        );
      if (operation.type === "render")
        return charactersApi.confirmRender(
          projectId,
          characterId,
          operation.generationId,
        );
      return charactersApi.confirmViews(
        projectId,
        characterId,
        operation.generationIds,
      );
    },
    onSuccess: async () => {
      message.success("人工门禁已确认");
      setSelectedViews([]);
      await refresh();
    },
    onError: (error: Error) => message.error(error.message),
  });

  const character = detail.data?.character;
  const generations = detail.data?.generations ?? [];
  const renderGenerations = generations.filter(
    (item) => item.stage === "render",
  );
  const viewGenerations = generations.filter((item) => item.stage === "views");
  const currentStep = Math.max(
    0,
    states.indexOf(character?.state ?? "S0_spec_drafting"),
  );
  const hasApprovedVerdict = (
    token: "SPEC-CHECK" | "VIEW-CHECK",
    notBefore: number,
  ) =>
    conversation.snapshot?.messages.some(
      (item) =>
        item.createdAt >= notBefore &&
        item.action?.payload.verdict?.token === token &&
        item.action.payload.verdict.decision === "APPROVE",
    ) ?? false;
  const hasCompleteViewSelection =
    selectedViews.length === 1 &&
    viewGenerations.some(
      (item) =>
        item.id === selectedViews[0] &&
        item.variant === "quad" &&
        hasApprovedVerdict("VIEW-CHECK", item.createdAt),
    );

  const renderDraftAction = (draft: ArtifactDraft, closeDrawer: () => void) =>
    character?.state === "S0_spec_drafting" &&
    draft.targetPath === "docs/角色定稿.md" ? (
      <Space size="small">
        <Button
          type="primary"
          size="small"
          icon={<CheckOutlined />}
          disabled={
            !canWrite || !hasApprovedVerdict("SPEC-CHECK", draft.createdAt)
          }
          loading={action.isPending}
          onClick={() =>
            action.mutate(
              { type: "spec", draftId: draft.id },
              { onSuccess: closeDrawer },
            )
          }
        >
          保存并确认设定
        </Button>
        <Button
          size="small"
          icon={<ReloadOutlined />}
          disabled={!canWrite || conversation.isBusy}
          onClick={() => {
            draftDrawerCloseRef.current = closeDrawer;
            setRejectOpen(true);
          }}
        >
          要求修改
        </Button>
      </Space>
    ) : null;

  const requestRedraw = async () => {
    const reason = rejectReason.trim();
    if (!reason || !character) return;
    const stage =
      character.state === "S0_spec_drafting"
        ? "spec"
        : character.state === "S2_render_generated"
          ? "render"
          : character.state === "S4_views_generated"
            ? "views"
            : null;
    if (!stage) return;
    setRequestingRedraw(true);
    try {
      if (stage === "spec")
        await charactersApi.rejectSpec(projectId, characterId, reason);
      if (stage === "render")
        await charactersApi.rejectRender(projectId, characterId, reason);
      if (stage === "views")
        await charactersApi.rejectViews(projectId, characterId, reason);
      await conversation.send(
        `用户已通过人工门禁拒绝当前 ${stage} 候选。必须按以下原因修订或重画：${reason}`,
        stage === "spec" ? "spec_writer" : "prompt_smith",
      );
      draftDrawerCloseRef.current?.();
      draftDrawerCloseRef.current = null;
      setRejectOpen(false);
      setRejectReason("");
      await refresh();
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setRequestingRedraw(false);
    }
  };

  return (
    <div className="page-stack workspace-page">
      <Card className="content-card">
        <Steps
          current={currentStep}
          items={labels.map((title) => ({ title }))}
        />
      </Card>
      <Row gutter={[16, 16]} align="stretch">
        <Col className="chat-column" xs={24} xl={15}>
          <ChatPanel
            snapshot={conversation.snapshot}
            agents={agents.data}
            loading={conversation.isLoading || detail.isLoading}
            canWrite={canWrite}
            busy={conversation.isBusy}
            interrupting={conversation.isInterrupting}
            streamingText={conversation.streamingText}
            thinkingText={conversation.thinkingText}
            workingAgentCode={conversation.workingAgentCode}
            lastError={conversation.lastError}
            starterPrompt={`帮我设计一个符合当前项目要求的角色，名字叫${character?.name ?? ""}`}
            onSend={conversation.send}
            onInterrupt={conversation.interrupt}
            onCommitDrafts={conversation.commitDrafts}
            renderDraftAction={renderDraftAction}
          />
        </Col>
        <Col xs={24} xl={9}>
          <Space orientation="vertical" className="workspace-main">
            <Card
              title="角色状态"
              className="content-card"
              extra={
                character && <Tag color="processing">{character.state}</Tag>
              }
            >
              <StatusRow label="角色" value={character?.name} />
              <StatusRow label="分组" value={character?.group ?? "未分组"} />
              <StatusRow label="目录" value={character?.dirName} />
              <StatusRow label="设定" value={character?.specPath} />
              <StatusRow label="效果图" value={character?.renderPath} />
              <StatusRow
                label="四视图"
                value={
                  character
                    ? Object.values(character.viewPaths).join("、")
                    : undefined
                }
              />
              <Typography.Text type="secondary">
                Agent 审校结论仅供参考，只有这里的人工操作会推进状态。
              </Typography.Text>
            </Card>
            {character?.state === "S2_render_generated" && (
              <GenerationGate
                title="选择效果图定稿"
                generations={renderGenerations}
                canWrite={canWrite}
                canConfirm={(generation) =>
                  hasApprovedVerdict("VIEW-CHECK", generation.createdAt)
                }
                onConfirm={(id) =>
                  action.mutate({ type: "render", generationId: id })
                }
                onReject={() => setRejectOpen(true)}
              />
            )}
            {character?.state === "S4_views_generated" && (
              <Card title="确认四视图" className="content-card">
                <Space orientation="vertical" className="workspace-main">
                  {viewGenerations.map((generation) => (
                    <Checkbox
                      key={generation.id}
                      checked={selectedViews.includes(generation.id)}
                      onChange={(event) =>
                        setSelectedViews(
                          event.target.checked ? [generation.id] : [],
                        )
                      }
                    >
                      {generation.variant === "quad"
                        ? "完整 2×2 四宫格"
                        : "格式不兼容"}{" "}
                      · {generation.filePath}
                    </Checkbox>
                  ))}
                  <Button
                    type="primary"
                    disabled={!canWrite || !hasCompleteViewSelection}
                    loading={action.isPending}
                    onClick={() =>
                      action.mutate({
                        type: "views",
                        generationIds: selectedViews,
                      })
                    }
                  >
                    确认完整四视图
                  </Button>
                  <Button
                    icon={<ReloadOutlined />}
                    disabled={!canWrite}
                    onClick={() => setRejectOpen(true)}
                  >
                    重画
                  </Button>
                </Space>
              </Card>
            )}
          </Space>
        </Col>
      </Row>

      <Modal
        title="说明拒绝原因"
        open={rejectOpen}
        okText="提交并继续修订"
        confirmLoading={requestingRedraw}
        okButtonProps={{
          disabled: !rejectReason.trim() || conversation.isBusy,
        }}
        onOk={() => void requestRedraw()}
        onCancel={() => {
          draftDrawerCloseRef.current = null;
          setRejectOpen(false);
        }}
      >
        <Input.TextArea
          value={rejectReason}
          autoSize={{ minRows: 3 }}
          placeholder="必须说明不采用当前结果的原因"
          onChange={(event) => setRejectReason(event.target.value)}
        />
      </Modal>
    </div>
  );
}

function StatusRow({ label, value }: { label: string; value?: string | null }) {
  return (
    <p>
      <Typography.Text strong>{label}：</Typography.Text>
      <Typography.Text type={value ? undefined : "secondary"}>
        {value || "未确认"}
      </Typography.Text>
    </p>
  );
}

function GenerationGate({
  title,
  generations,
  canWrite,
  canConfirm,
  onConfirm,
  onReject,
}: {
  title: string;
  generations: Generation[];
  canWrite: boolean;
  canConfirm: (generation: Generation) => boolean;
  onConfirm: (id: string) => void;
  onReject: () => void;
}) {
  return (
    <Card title={title} className="content-card">
      <Space orientation="vertical" className="workspace-main">
        {generations.map((generation) => (
          <Card
            size="small"
            key={generation.id}
            title={generation.variant ?? "候选"}
          >
            <Typography.Text className="path-text">
              {generation.filePath}
            </Typography.Text>
            <Button
              type="primary"
              size="small"
              disabled={!canWrite || !canConfirm(generation)}
              onClick={() => onConfirm(generation.id)}
            >
              采用
            </Button>
          </Card>
        ))}
        {!generations.length && (
          <Typography.Text type="secondary">尚无可确认候选</Typography.Text>
        )}
        <Button
          icon={<ReloadOutlined />}
          disabled={!canWrite || !generations.length}
          onClick={onReject}
        >
          重画
        </Button>
      </Space>
    </Card>
  );
}
