import {
  DeleteOutlined,
  PlusOutlined,
  RobotOutlined,
  SettingOutlined,
} from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Descriptions,
  Empty,
  Form,
  Input,
  InputNumber,
  List,
  Modal,
  Select,
  Space,
  Table,
  Tag,
  Typography,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useMemo, useState } from "react";
import { aiApi } from "./api";
import { useStudio } from "./AppShell";
import type { AiAgent, AiLimit, AiModel, AiProvider } from "./types";

const roleTypeLabels: Record<AiAgent["roleType"], string> = {
  director: "总管",
  specialist: "专家",
  executor: "执行器",
};
const limitKinds = [
  "calls",
  "images",
  "input_tokens",
  "output_tokens",
  "total_tokens",
  "tokens",
  "credits",
  "duration_seconds",
];
const limitKindLabels: Record<string, string> = {
  calls: "调用次数",
  images: "图片张数",
  input_tokens: "输入 Token",
  output_tokens: "输出 Token",
  total_tokens: "总 Token",
  tokens: "Token",
  credits: "Credits",
  duration_seconds: "语音时长（秒）",
};
const periodOptions = [
  "second",
  "minute",
  "hour",
  "day",
  "week",
  "month",
  "total",
  "day+11H",
].map((value) => ({ value, label: value }));

export default function AgentsPage() {
  const { canWrite } = useStudio();
  const agents = useQuery({
    queryKey: ["ai-agents"],
    queryFn: aiApi.listAgents,
  });
  const providers = useQuery({
    queryKey: ["ai-providers"],
    queryFn: aiApi.listProviders,
  });
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<AiAgent>();

  const columns: ColumnsType<AiAgent> = [
    {
      title: "Agent",
      render: (_, agent) => (
        <Space direction="vertical" size={0}>
          <Space>
            <RobotOutlined />
            <Typography.Text strong>{agent.role}</Typography.Text>
          </Space>
          <Typography.Text type="secondary" code>
            {agent.agentCode}
          </Typography.Text>
        </Space>
      ),
    },
    {
      title: "类型",
      width: 110,
      render: (_, agent) => <Tag>{roleTypeLabels[agent.roleType]}</Tag>,
    },
    {
      title: "能力",
      width: 190,
      render: (_, agent) => (
        <Space direction="vertical" size={0}>
          <Tag color="blue">{agent.capability}</Tag>
          <Typography.Text type="secondary">
            {agent.requiredModelCapability}
          </Typography.Text>
        </Space>
      ),
    },
    {
      title: "目标 / 阶段",
      render: (_, agent) => (
        <Space wrap size={[4, 4]}>
          {agent.targetKinds.map((item) => (
            <Tag key={`target-${item}`}>{item}</Tag>
          ))}
          {agent.stages.map((item) => (
            <Tag color="purple" key={`stage-${item}`}>
              {item}
            </Tag>
          ))}
          {!agent.stages.length && (
            <Typography.Text type="secondary">未接入流程</Typography.Text>
          )}
        </Space>
      ),
    },
    {
      title: "状态",
      width: 130,
      render: (_, agent) =>
        agent.modelIds.length ? (
          <Tag color="success">{agent.modelIds.length} 个模型</Tag>
        ) : (
          <Tag color="warning">未配置模型</Tag>
        ),
    },
    {
      title: "操作",
      width: 120,
      render: (_, agent) => (
        <Button
          icon={<SettingOutlined />}
          disabled={!canWrite}
          onClick={() => setEditing(agent)}
        >
          指定模型
        </Button>
      ),
    },
  ];

  return (
    <div className="page-stack">
      <section className="page-heading">
        <div>
          <Typography.Title level={2}>Agent 配置</Typography.Title>
          <Typography.Text type="secondary">
            角色和提示词来自 bundled markdown，只能配置兼容模型及有序回退顺序。
          </Typography.Text>
        </div>
      </section>

      <Table
        className="content-card"
        columns={columns}
        dataSource={agents.data ?? []}
        loading={agents.isLoading}
        rowKey="agentCode"
        locale={{ emptyText: <Empty description="没有 bundled Agent" /> }}
        expandable={{
          expandedRowRender: (agent) => <AgentMetadata agent={agent} />,
        }}
        pagination={false}
      />

      <AgentBindingModal
        agent={editing}
        providers={providers.data ?? []}
        canWrite={canWrite}
        onClose={() => setEditing(undefined)}
        onSaved={async () => {
          await queryClient.invalidateQueries({ queryKey: ["ai-agents"] });
          setEditing(undefined);
        }}
      />
    </div>
  );
}

function AgentMetadata({ agent }: { agent: AiAgent }) {
  return (
    <Descriptions size="small" column={{ xs: 1, md: 2, xl: 3 }}>
      <Descriptions.Item label="别名">
        {agent.aliases.join("、") || "无"}
      </Descriptions.Item>
      <Descriptions.Item label="可对焦">
        {agent.focusable ? "是" : "否"}
      </Descriptions.Item>
      <Descriptions.Item label="可对话">
        {agent.conversational ? "是" : "否"}
      </Descriptions.Item>
      <Descriptions.Item label="最大轮次">{agent.maxTurns}</Descriptions.Item>
      <Descriptions.Item label="记忆范围">
        {agent.memoryScope}
      </Descriptions.Item>
      <Descriptions.Item label="上下文预算">
        {agent.contextBudget.toLocaleString()}
      </Descriptions.Item>
      <Descriptions.Item label="最大输出">
        {agent.maxOutputTokens?.toLocaleString() ?? "模型默认"}
      </Descriptions.Item>
      <Descriptions.Item label="输出契约">
        {agent.outputContract}
      </Descriptions.Item>
      <Descriptions.Item label="工具白名单">
        {agent.allowTools.join("、") || "无"}
      </Descriptions.Item>
      <Descriptions.Item label="源文件" span={3}>
        <Typography.Text type="secondary" className="path-text">
          {agent.sourceFile}
        </Typography.Text>
      </Descriptions.Item>
    </Descriptions>
  );
}

type AgentBindingModalProps = {
  agent?: AiAgent;
  providers: AiProvider[];
  canWrite: boolean;
  onClose: () => void;
  onSaved: () => Promise<void>;
};

function AgentBindingModal({
  agent,
  providers,
  canWrite,
  onClose,
  onSaved,
}: AgentBindingModalProps) {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [limitForm] = Form.useForm<{ limits: AiLimit[] }>();
  const [modelIds, setModelIds] = useState<string[]>([]);
  const [providerCode, setProviderCode] = useState<string>();
  const [candidateId, setCandidateId] = useState<string>();
  const [limitTarget, setLimitTarget] = useState<{
    provider: AiProvider;
    model: AiModel;
  }>();

  useEffect(() => {
    setModelIds(agent?.modelIds ?? []);
    setProviderCode(undefined);
    setCandidateId(undefined);
    setLimitTarget(undefined);
  }, [agent]);

  const modelMap = useMemo(
    () =>
      new Map(
        providers.flatMap((provider) =>
          provider.models.map(
            (model) => [model.id, { provider, model }] as const,
          ),
        ),
      ),
    [providers],
  );
  const candidates = useMemo(() => {
    if (!agent || !providerCode) return [];
    const provider = providers.find((item) => item.code === providerCode);
    if (!provider) return [];
    return provider.models
      .filter(
        (model) =>
          model.enabled &&
          !modelIds.includes(model.id) &&
          model.capabilities.includes(agent.requiredModelCapability),
      )
      .sort((left, right) => {
        if (agent.capability !== "i2i") return left.sortNo - right.sortNo;
        const leftScore = Number(
          left.capabilities.includes("image_reference_consistency"),
        );
        const rightScore = Number(
          right.capabilities.includes("image_reference_consistency"),
        );
        return rightScore - leftScore || left.sortNo - right.sortNo;
      });
  }, [agent, modelIds, providerCode, providers]);

  const save = useMutation({
    mutationFn: () => aiApi.writeAgentBinding(agent!.agentCode, modelIds),
    onSuccess: async () => {
      message.success(`${agent!.role} 的模型顺序已保存`);
      await onSaved();
    },
    onError: (error: Error) => message.error(error.message),
  });

  const saveLimits = useMutation({
    mutationFn: async (values: { limits: AiLimit[] }) => {
      if (!limitTarget) throw new Error("未选择模型");
      return aiApi.writeModel("update", {
        ...limitTarget.model,
        limits: values.limits ?? [],
      });
    },
    onSuccess: async () => {
      message.success("模型限流已保存");
      setLimitTarget(undefined);
      await queryClient.invalidateQueries({ queryKey: ["ai-providers"] });
    },
    onError: (error: Error) => message.error(error.message),
  });

  const openLimits = (provider: AiProvider, model: AiModel) => {
    setLimitTarget({ provider, model });
    limitForm.setFieldsValue({ limits: model.limits });
  };

  const sortedModelIds = (ids: string[]) =>
    [...ids].sort((leftId, rightId) => {
      const left = modelMap.get(leftId);
      const right = modelMap.get(rightId);
      if (!left || !right) return 0;
      return (
        left.provider.priority - right.provider.priority ||
        left.model.sortNo - right.model.sortNo
      );
    });

  return (
    <>
      <Modal
        title={agent ? `指定模型 · ${agent.role}` : "指定模型"}
        open={Boolean(agent)}
        onCancel={onClose}
        onOk={() => save.mutate()}
        okText="保存"
        okButtonProps={{ disabled: !canWrite, loading: save.isPending }}
        width="clamp(720px, 55vw, 960px)"
        destroyOnHidden
      >
        <Space direction="vertical" className="workspace-main" size="middle">
          <Typography.Text type="secondary">
            仅显示已启用且支持 {agent?.requiredModelCapability ?? "所需能力"}{" "}
            的模型；调用顺序按 Provider 优先级和模型排序确定。
          </Typography.Text>
          <List
            bordered
            locale={{ emptyText: "未绑定模型，执行时将返回 blocked" }}
            dataSource={modelIds}
            renderItem={(id, index) => {
              const entry = modelMap.get(id);
              return (
                <List.Item
                  actions={[
                    entry && (
                      <Button
                        key="limit"
                        type="link"
                        size="small"
                        onClick={() => openLimits(entry.provider, entry.model)}
                      >
                        限流
                      </Button>
                    ),
                    <Button
                      key="remove"
                      type="text"
                      danger
                      icon={<DeleteOutlined />}
                      onClick={() =>
                        setModelIds((current) =>
                          current.filter((item) => item !== id),
                        )
                      }
                    />,
                  ]}
                >
                  <List.Item.Meta
                    title={
                      <Space>
                        <Tag color="geekblue">#{index + 1}</Tag>
                        {entry?.model.displayName ?? id}
                      </Space>
                    }
                    description={
                      entry ? (
                        <Space direction="vertical" size={4}>
                          <Typography.Text type="secondary">
                            {entry.provider.name} · {entry.model.modelId}
                          </Typography.Text>
                          <Space wrap size={[4, 4]}>
                            {entry.model.limits.length ? (
                              entry.model.limits.map((limit) => (
                                <Tag
                                  key={`${limit.limitKind}:${limit.groupName}`}
                                >
                                  {limitKindLabels[limit.limitKind] ??
                                    limit.limitKind}
                                  ：
                                  {limit.maxValue > 0
                                    ? `${limit.maxValue} / ${limit.periodExpr}`
                                    : "不限"}
                                  {limit.groupName !== "default"
                                    ? ` · ${limit.groupName}`
                                    : ""}
                                </Tag>
                              ))
                            ) : (
                              <Tag>未设置限流</Tag>
                            )}
                          </Space>
                        </Space>
                      ) : (
                        "模型已不存在，请移除后保存"
                      )
                    }
                  />
                </List.Item>
              );
            }}
          />
          <Space direction="vertical" size={4} className="workspace-main">
            <Typography.Text type="secondary">
              第一步：选择 Provider
            </Typography.Text>
            <Select
              value={providerCode}
              allowClear
              showSearch
              optionFilterProp="label"
              placeholder="选择 Provider"
              style={{ width: "100%" }}
              options={providers.map((provider) => ({
                value: provider.code,
                label: provider.enabled
                  ? provider.name
                  : `${provider.name}（已停用）`,
                disabled: !provider.enabled,
              }))}
              onChange={(value) => {
                setProviderCode(value);
                setCandidateId(undefined);
              }}
            />
            <Typography.Text type="secondary">第二步：选择模型</Typography.Text>
            <Space.Compact block>
              <Select
                value={candidateId}
                showSearch
                optionFilterProp="label"
                placeholder={
                  providerCode ? "选择兼容模型" : "请先选择 Provider"
                }
                disabled={!providerCode}
                style={{ width: "100%" }}
                options={candidates.map((model) => ({
                  value: model.id,
                  label: `${model.displayName} (${model.modelId})`,
                }))}
                onChange={setCandidateId}
              />
              <Button
                icon={<PlusOutlined />}
                disabled={!candidateId}
                onClick={() => {
                  if (!candidateId) return;
                  setModelIds((current) =>
                    sortedModelIds([...current, candidateId]),
                  );
                  setCandidateId(undefined);
                }}
              >
                添加
              </Button>
            </Space.Compact>
          </Space>
        </Space>
      </Modal>

      <Modal
        width="clamp(760px, 56vw, 980px)"
        title={
          limitTarget
            ? `${limitTarget.provider.name} · ${limitTarget.model.displayName} · 模型限流`
            : "模型限流"
        }
        open={Boolean(limitTarget)}
        confirmLoading={saveLimits.isPending}
        onCancel={() => setLimitTarget(undefined)}
        onOk={() => limitForm.submit()}
        destroyOnHidden
      >
        <Form
          form={limitForm}
          layout="vertical"
          onFinish={(values) => saveLimits.mutate(values)}
        >
          <AgentLimitFields />
        </Form>
      </Modal>
    </>
  );
}

function AgentLimitFields() {
  return (
    <Form.List name="limits">
      {(fields, { add, remove }) => (
        <Space direction="vertical" className="workspace-main">
          {fields.map((field) => (
            <Space key={field.key} align="start" wrap>
              <Form.Item
                name={[field.name, "limitKind"]}
                rules={[{ required: true }]}
              >
                <Select
                  placeholder="限流口径"
                  options={limitKinds.map((value) => ({
                    value,
                    label: limitKindLabels[value] ?? value,
                  }))}
                  style={{ width: 160 }}
                />
              </Form.Item>
              <Form.Item
                name={[field.name, "maxValue"]}
                rules={[{ required: true }]}
              >
                <InputNumber min={0} placeholder="0 为不限" />
              </Form.Item>
              <Form.Item
                name={[field.name, "periodExpr"]}
                rules={[{ required: true }]}
              >
                <Select
                  showSearch
                  placeholder="限流周期"
                  options={periodOptions}
                  style={{ width: 150 }}
                />
              </Form.Item>
              <Form.Item
                name={[field.name, "groupName"]}
                rules={[{ required: true }]}
              >
                <Input placeholder="共享组" />
              </Form.Item>
              <Button
                danger
                type="text"
                icon={<DeleteOutlined />}
                onClick={() => remove(field.name)}
              />
            </Space>
          ))}
          <Button
            type="dashed"
            icon={<PlusOutlined />}
            onClick={() =>
              add({
                limitKind: "calls",
                maxValue: 0,
                periodExpr: "day",
                groupName: "default",
              })
            }
          >
            增加限流
          </Button>
        </Space>
      )}
    </Form.List>
  );
}

export function modelStatus(model: AiModel, provider: AiProvider) {
  if (!provider.enabled) return "Provider 已停用";
  if (!model.enabled) return "模型已停用";
  if (!provider.hasKey) return "缺少 API Key";
  return "可用";
}
