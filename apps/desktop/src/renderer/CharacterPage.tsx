import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Card,
  Col,
  Image,
  Modal,
  Popconfirm,
  Progress,
  Row,
  Select,
  Space,
  Spin,
  Steps,
  Tag,
  Typography,
} from "antd";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useParams } from "react-router-dom";
import { aiApi, charactersApi, workspaceApi } from "./api";
import { useStudio } from "./AppShell";
import ChatPanel from "./chat/ChatPanel";
import MarkdownDocument from "./chat/MarkdownDocument";
import { useConversation } from "./chat/useConversation";
import type { ArtifactDraft, FeedbackImage, Generation } from "./types";

const characterStateLabels: Record<string, string> = {
  S0_spec_drafting: "角色设定中",
  S1_spec_confirmed: "角色设定已确认",
  S2_render_generated: "效果图待确认",
  S3_render_confirmed: "效果图已确认",
  S4_views_generated: "四视图待确认",
  S5_views_confirmed: "角色视觉设计完成",
};

const model3dStages: Record<string, { label: string; percent: number }> = {
  uploadingViews: { label: "上传四视图", percent: 10 },
  generatingModel: { label: "生成低面模型", percent: 30 },
  checkingRig: { label: "检查可绑骨性", percent: 45 },
  rigging: { label: "绑定 Mixamo 骨架", percent: 60 },
  retargeting: { label: "烘焙预设动画", percent: 78 },
  downloading: { label: "下载 GLB", percent: 90 },
  validating: { label: "校验 GLB", percent: 96 },
  completed: { label: "生成完成", percent: 100 },
};

export default function CharacterPage() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const { projectId = "", characterId = "" } = useParams();
  const { canWrite, setActiveProject } = useStudio();
  const resumedContinuationKeys = useRef(new Set<string>());
  const [specPreviewOpen, setSpecPreviewOpen] = useState(false);
  const [model3dProviderCode, setModel3dProviderCode] = useState<string>();

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
  const model3dProviders = useQuery({
    queryKey: ["model3d-providers"],
    queryFn: charactersApi.listModel3dProviders,
  });
  const model3d = useQuery({
    queryKey: ["character-model3d", projectId, characterId],
    queryFn: () => charactersApi.readModel3d(projectId, characterId),
    enabled: Boolean(projectId && characterId),
    refetchInterval: (query) =>
      ["pending", "running"].includes(query.state.data?.status ?? "")
        ? 1500
        : false,
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
    if (model3dProviderCode) return;
    const provider = model3dProviders.data?.find((item) => item.hasKey);
    if (provider) setModel3dProviderCode(provider.code);
  }, [model3dProviderCode, model3dProviders.data]);

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
      queryClient.invalidateQueries({
        queryKey: ["character-model3d", projectId, characterId],
      }),
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
      await refresh();
    },
  });

  const startModel3d = useMutation({
    mutationFn: () => {
      if (!model3dProviderCode) throw new Error("没有已配置密钥的 3D 模型 Provider");
      return charactersApi.startModel3d(
        projectId,
        characterId,
        model3dProviderCode,
      );
    },
    onSuccess: (job) => {
      queryClient.setQueryData(
        ["character-model3d", projectId, characterId],
        job,
      );
      message.success("3D 游戏模型任务已启动");
    },
    onError: (error: Error) => message.error(error.message),
  });

  const character = detail.data?.character;
  const generations = detail.data?.generations ?? [];
  const publishedRender = generations.find(
    (item) => item.stage === "render" && item.isFinal,
  );
  const publishedViews = generations.find(
    (item) => item.stage === "views" && item.isFinal,
  );
  const pendingRenderGenerations = generations.filter(
    (item) => item.stage === "render" && item.reviewStatus === "pending",
  );
  const pendingViewGenerations = generations.filter(
    (item) => item.stage === "views" && item.reviewStatus === "pending",
  );
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
  const pendingMediaGenerations = isRenderConfirmation
    ? pendingRenderGenerations
    : isViewsConfirmation
      ? pendingViewGenerations
      : [];
  const model3dJob = model3d.data;
  const model3dStage = model3dJob
    ? model3dStages[model3dJob.stage]
    : undefined;
  const model3dRunning =
    model3dJob?.status === "pending" || model3dJob?.status === "running";

  useEffect(() => {
    const workflowProgress = detail.data?.workflowProgress;
    const continuationKey = workflowProgress?.continuationKey;
    if (!canWrite || !workflowProgress?.needsResume || !continuationKey) return;
    const resumeKey = `${projectId}:${characterId}:${continuationKey}`;
    if (resumedContinuationKeys.current.has(resumeKey)) return;
    resumedContinuationKeys.current.add(resumeKey);
    void charactersApi
      .resume(projectId, characterId, continuationKey)
      .then(async () => {
        await Promise.all([
          queryClient.invalidateQueries({
            queryKey: ["character", projectId, characterId],
          }),
          queryClient.invalidateQueries({ queryKey: ["characters", projectId] }),
          conversation.refresh(),
        ]);
      })
      .catch((error: unknown) => {
        resumedContinuationKeys.current.delete(resumeKey);
        message.error(
          error instanceof Error ? error.message : "自动恢复角色工作流失败",
        );
      });
  }, [
    canWrite,
    characterId,
    conversation,
    detail.data?.workflowProgress,
    message,
    projectId,
    queryClient,
  ]);

  const confirmSpecDraft = async (draft: ArtifactDraft) => {
    if (
      character?.state !== "S0_spec_drafting" ||
      draft.targetPath !== "docs/角色定稿.md"
    ) {
      throw new Error("当前草稿不是待确认的角色设定");
    }
    await action.mutateAsync({ type: "spec", draftId: draft.id });
  };
  const confirmMedia = async (generation: Generation) => {
    if (!character || generation.reviewStatus !== "pending") {
      throw new Error("当前生成结果已变化，请刷新后重试");
    }
    if (
      (generation.stage === "render" && character.state !== "S2_render_generated") ||
      (generation.stage === "views" && character.state !== "S4_views_generated")
    ) {
      throw new Error("当前角色阶段已变化，请刷新后重试");
    }
    if (generation.stage === "views" && generation.variant !== "quad") {
      throw new Error("请选择一张完整四视图");
    }
    await action.mutateAsync(
      generation.stage === "render"
        ? { type: "render", generationId: generation.id }
        : { type: "views", generationIds: [generation.id] },
    );
  };
  const requestSpecRevision = async (content: string) => {
    if (character?.state !== "S0_spec_drafting") {
      throw new Error("当前角色阶段已变化，请刷新后重试");
    }
    await charactersApi.rejectSpec(projectId, characterId, content);
    await refresh();
  };
  const requestMediaRevision = async (
    generation: Generation,
    content: string,
    image?: FeedbackImage,
  ) => {
    if (!character || generation.reviewStatus !== "pending") {
      throw new Error("当前生成结果已变化，请刷新后重试");
    }
    await charactersApi.requestGenerationRevision(
      projectId,
      characterId,
      generation.id,
      content,
      image,
    );
    await refresh();
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
              isSpecConfirmation ? requestSpecRevision : undefined
            }
            mediaGenerations={pendingMediaGenerations}
            onConfirmMedia={confirmMedia}
            onSubmitMediaFeedback={requestMediaRevision}
            confirmingMedia={action.isPending}
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
              <StatusRow label="设定">
                {detail.data?.specMarkdown ? (
                  <Typography.Link onClick={() => setSpecPreviewOpen(true)}>
                    查看设定文档
                  </Typography.Link>
                ) : null}
              </StatusRow>
              <StatusRow label="效果图">
                {publishedRender ? (
                  <PublishedGenerationThumbnail
                    generation={publishedRender}
                    alt="角色效果图"
                  />
                ) : null}
              </StatusRow>
              <StatusRow label="四视图">
                {publishedViews ? (
                  <PublishedGenerationThumbnail
                    generation={publishedViews}
                    alt="角色四视图"
                  />
                ) : null}
              </StatusRow>
              <Typography.Text type="secondary">
                Agent 审校结论仅供参考，只有这里的人工操作会推进状态。
              </Typography.Text>
            </Card>
            <Card
              title="3D 游戏模型"
              className="content-card"
              extra={
                <Space>
                  {(model3dProviders.data?.length ?? 0) > 1 && (
                    <Select
                      value={model3dProviderCode}
                      options={model3dProviders.data?.map((provider) => ({
                        value: provider.code,
                        label: provider.name,
                        disabled: !provider.hasKey,
                      }))}
                      onChange={setModel3dProviderCode}
                    />
                  )}
                  <Popconfirm
                    title="上次请求结果不确定"
                    description="上次提交可能已在远端计费。已完成的步骤会跳过，确认重试？"
                    okText="确认重试"
                    cancelText="取消"
                    disabled={model3dJob?.status !== "needsAttention"}
                    onConfirm={() => startModel3d.mutate()}
                  >
                    <Button
                      type="primary"
                      disabled={
                        !canWrite ||
                        !model3dProviderCode ||
                        character?.state !== "S5_views_confirmed" ||
                        model3dRunning ||
                        model3dJob?.status === "succeeded"
                      }
                      loading={startModel3d.isPending}
                      onClick={() => {
                        if (model3dJob?.status !== "needsAttention") {
                          startModel3d.mutate();
                        }
                      }}
                    >
                      {model3dJob?.status === "failed" ||
                      model3dJob?.status === "needsAttention"
                        ? "重新生成"
                        : "生成 3D 模型"}
                    </Button>
                  </Popconfirm>
                </Space>
              }
            >
              {model3d.isLoading ? (
                <Spin size="small" />
              ) : model3dJob ? (
                <Space orientation="vertical" className="workspace-main">
                  <Tag
                    color={
                      model3dJob.status === "succeeded"
                        ? "success"
                        : model3dJob.status === "failed" ||
                            model3dJob.status === "needsAttention"
                          ? "error"
                          : "processing"
                    }
                  >
                    {model3dStage?.label ?? model3dJob.status}
                  </Tag>
                  {model3dRunning && (
                    <Progress percent={model3dStage?.percent ?? 0} />
                  )}
                  {model3dJob.asset && (
                    <>
                      <StatusRow label="GLB" value={model3dJob.asset.path} />
                      <StatusRow
                        label="动画"
                        value={model3dJob.asset.animationClips.join("、")}
                      />
                      <StatusRow
                        label="大小"
                        value={`${(model3dJob.asset.bytes / 1024 / 1024).toFixed(1)} MiB`}
                      />
                    </>
                  )}
                  {model3dJob.error && (
                    <Typography.Text type="danger">
                      {model3dJob.error}
                    </Typography.Text>
                  )}
                </Space>
              ) : (
                <Typography.Text type="secondary">
                  确认四视图后，可手动生成 P1 低面、Mixamo 骨架及预设动画 GLB。
                </Typography.Text>
              )}
            </Card>
            <Modal
              open={specPreviewOpen}
              title={`${character?.name ?? "角色"}设定`}
              footer={null}
              width={760}
              onCancel={() => setSpecPreviewOpen(false)}
            >
              {detail.data?.specMarkdown && (
                <MarkdownDocument content={detail.data.specMarkdown} />
              )}
            </Modal>
          </Space>
        </Col>
      </Row>
    </div>
  );
}

function StatusRow({
  label,
  value,
  children,
}: {
  label: string;
  value?: string | null;
  children?: ReactNode;
}) {
  const content = children ?? value;
  return (
    <div className="character-status-row">
      <Typography.Text strong>{label}：</Typography.Text>
      {content ? (
        content
      ) : (
        <Typography.Text type="secondary">未确认</Typography.Text>
      )}
    </div>
  );
}

function PublishedGenerationThumbnail({
  generation,
  alt,
}: {
  generation: Generation;
  alt: string;
}) {
  const media = useQuery({
    queryKey: ["generation-media", generation.projectId, generation.id],
    queryFn: () => charactersApi.readGenerationMedia(generation.projectId, generation.id),
    staleTime: Number.POSITIVE_INFINITY,
  });
  if (media.isLoading) return <Spin size="small" />;
  if (!media.data || media.error) {
    return <Typography.Text type="secondary">图片加载失败</Typography.Text>;
  }
  const dataUrl = `data:${media.data.mimeType};base64,${media.data.dataBase64}`;
  return (
    <Image
      className="character-asset-thumbnail"
      src={dataUrl}
      alt={alt}
      width={112}
      height={112}
      preview={{ mask: "查看大图" }}
    />
  );
}
