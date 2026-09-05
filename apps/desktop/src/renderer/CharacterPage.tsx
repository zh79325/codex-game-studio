import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Card,
  Checkbox,
  Col,
  Radio,
  Row,
  Space,
  Steps,
  Tag,
  Typography,
} from "antd";
import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import { aiApi, charactersApi, workspaceApi } from "./api";
import { useStudio } from "./AppShell";
import ChatPanel from "./chat/ChatPanel";
import FinalConfirmationActions from "./chat/FinalConfirmationActions";
import { useConversation } from "./chat/useConversation";
import type { ArtifactDraft, Generation } from "./types";

const characterStateLabels: Record<string, string> = {
  S0_spec_drafting: "角色设定中",
  S1_spec_confirmed: "角色设定已确认",
  S2_render_generated: "效果图待确认",
  S3_render_confirmed: "效果图已确认",
  S4_views_generated: "四视图待确认",
  S5_views_confirmed: "角色视觉设计完成",
};

export default function CharacterPage() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const { projectId = "", characterId = "" } = useParams();
  const { canWrite, setActiveProject } = useStudio();
  const [selectedRender, setSelectedRender] = useState<string>();
  const [selectedViews, setSelectedViews] = useState<string[]>([]);

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

  useEffect(() => {
    if (!conversation.snapshot) return;
    void queryClient.invalidateQueries({
      queryKey: ["character", projectId, characterId],
    });
  }, [
    characterId,
    conversation.snapshot?.conversation.updatedAt,
    projectId,
    queryClient,
  ]);

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
      setSelectedRender(undefined);
      setSelectedViews([]);
      await refresh();
    },
  });

  const character = detail.data?.character;
  const generations = detail.data?.generations ?? [];
  const renderGenerations = generations.filter(
    (item) => item.stage === "render",
  );
  const viewGenerations = generations.filter((item) => item.stage === "views");
  const workflowSteps = detail.data?.workflowProgress.steps ?? [];
  const currentStep = workflowSteps.findIndex(
    (step) => step.status === "process" || step.status === "error",
  );
  const isSpecConfirmation = workflowSteps.some(
    (step) => step.key === "spec_confirm" && step.status === "process",
  );
  const isRenderConfirmation = workflowSteps.some(
    (step) => step.key === "render_confirm" && step.status === "process",
  );
  const isViewsConfirmation = workflowSteps.some(
    (step) => step.key === "views_confirm" && step.status === "process",
  );
  const hasCompleteViewSelection =
    selectedViews.length === 1 &&
    viewGenerations.some(
      (item) => item.id === selectedViews[0] && item.variant === "quad",
    );
  const hasRenderSelection = renderGenerations.some(
    (item) => item.id === selectedRender,
  );

  const continueAfterConfirmation = async (
    stage: "spec" | "render" | "views",
  ) => {
    const stageLabel =
      stage === "spec" ? "角色设定" : stage === "render" ? "效果图" : "四视图";
    await conversation.send(
      `用户已确认当前角色的${stageLabel}。请根据已确认结果决定并推进下一步。`,
      "studio_director",
    );
  };
  const confirmSpecDraft = async (draft: ArtifactDraft) => {
    if (
      character?.state !== "S0_spec_drafting" ||
      draft.targetPath !== "docs/角色定稿.md"
    ) {
      throw new Error("当前草稿不是待确认的角色设定");
    }
    await action.mutateAsync({ type: "spec", draftId: draft.id });
    await continueAfterConfirmation("spec");
  };
  const confirmRender = async () => {
    if (!selectedRender || !hasRenderSelection) {
      throw new Error("请先选择一张效果图候选");
    }
    await action.mutateAsync({
      type: "render",
      generationId: selectedRender,
    });
    await continueAfterConfirmation("render");
  };
  const confirmViews = async () => {
    if (!hasCompleteViewSelection) {
      throw new Error("请先选择一张完整四视图");
    }
    await action.mutateAsync({
      type: "views",
      generationIds: selectedViews,
    });
    await continueAfterConfirmation("views");
  };
  const requestRevision = async (
    stage: "spec" | "render" | "views",
    content: string,
  ) => {
    if (!character) throw new Error("角色信息尚未加载");
    const expectedState =
      stage === "spec"
        ? "S0_spec_drafting"
        : stage === "render"
          ? "S2_render_generated"
          : "S4_views_generated";
    if (character.state !== expectedState) {
      throw new Error("当前角色阶段已变化，请刷新后重试");
    }
    if (stage === "spec") {
      await charactersApi.rejectSpec(projectId, characterId, content);
    } else if (stage === "render") {
      await charactersApi.rejectRender(projectId, characterId, content);
      setSelectedRender(undefined);
    } else {
      await charactersApi.rejectViews(projectId, characterId, content);
      setSelectedViews([]);
    }
    await refresh();
    await conversation.send(
      `用户对当前角色${stage === "spec" ? "设定" : stage === "render" ? "效果图" : "四视图"}有以下补充要求：${content}`,
      "studio_director",
    );
  };

  return (
    <div className="page-stack workspace-page">
      <Card className="content-card">
        <Steps
          current={currentStep < 0 ? workflowSteps.length : currentStep}
          items={workflowSteps.map((step) => ({
            key: step.key,
            title: step.label,
            status: step.status,
          }))}
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
            onConfirmDraft={isSpecConfirmation ? confirmSpecDraft : undefined}
            confirmingDraft={action.isPending}
            onSubmitDraftFeedback={
              isSpecConfirmation
                ? (content) => requestRevision("spec", content)
                : undefined
            }
          />
        </Col>
        <Col xs={24} xl={9}>
          <Space orientation="vertical" className="workspace-main">
            <Card
              title="角色状态"
              className="content-card"
              extra={
                character && (
                  <Tag color="processing">
                    {detail.data?.workflowProgress.statusLabel ??
                      characterStateLabels[character.state]}
                  </Tag>
                )
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
            {isRenderConfirmation && (
              <GenerationGate
                title="选择效果图定稿"
                generations={renderGenerations}
                selectedId={selectedRender}
                disabled={!canWrite || conversation.isBusy}
                confirming={action.isPending}
                onSelect={setSelectedRender}
                onConfirm={confirmRender}
                onSupplement={(content) => requestRevision("render", content)}
              />
            )}
            {isViewsConfirmation && (
              <Card title="确认四视图" className="content-card">
                <Space orientation="vertical" className="workspace-main">
                  {viewGenerations.map((generation) => (
                    <Checkbox
                      key={generation.id}
                      checked={selectedViews.includes(generation.id)}
                      disabled={
                        !canWrite ||
                        conversation.isBusy ||
                        generation.variant !== "quad"
                      }
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
                  <FinalConfirmationActions
                    confirming={action.isPending}
                    onConfirm={confirmViews}
                    onSupplement={(content) => requestRevision("views", content)}
                  />
                </Space>
              </Card>
            )}
          </Space>
        </Col>
      </Row>
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
  selectedId,
  disabled,
  confirming,
  onSelect,
  onConfirm,
  onSupplement,
}: {
  title: string;
  generations: Generation[];
  selectedId?: string;
  disabled: boolean;
  confirming: boolean;
  onSelect: (id: string) => void;
  onConfirm: () => Promise<unknown>;
  onSupplement: (content: string) => Promise<unknown>;
}) {
  return (
    <Card title={title} className="content-card">
      <Space orientation="vertical" className="workspace-main">
        <Radio.Group
          className="generation-options"
          value={selectedId}
          disabled={disabled}
          onChange={(event) => onSelect(event.target.value as string)}
        >
          {generations.map((generation) => (
            <Card size="small" key={generation.id}>
              <Radio value={generation.id}>
                {generation.variant ?? "候选"}
              </Radio>
              <Typography.Text className="path-text">
                {generation.filePath}
              </Typography.Text>
            </Card>
          ))}
        </Radio.Group>
        {!generations.length && (
          <Typography.Text type="secondary">尚无可确认候选</Typography.Text>
        )}
        <FinalConfirmationActions
          confirming={confirming}
          onConfirm={onConfirm}
          onSupplement={onSupplement}
        />
      </Space>
    </Card>
  );
}
