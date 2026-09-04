import {
  ArrowDownOutlined,
  ArrowUpOutlined,
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
import type { AiAgent, AiModel, AiProvider } from "./types";

const roleTypeLabels: Record<AiAgent["roleType"], string> = {
  director: "总管",
  specialist: "专家",
  executor: "执行器",
};

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
  const [modelIds, setModelIds] = useState<string[]>([]);
  const [candidateId, setCandidateId] = useState<string>();

  useEffect(() => {
    setModelIds(agent?.modelIds ?? []);
    setCandidateId(undefined);
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
    if (!agent) return [];
    return providers
      .filter((provider) => provider.enabled)
      .flatMap((provider) =>
        provider.models.map((model) => ({ provider, model })),
      )
      .filter(
        ({ model }) =>
          model.enabled &&
          !modelIds.includes(model.id) &&
          model.capabilities.includes(agent.requiredModelCapability),
      )
      .sort((left, right) => {
        if (agent.capability !== "i2i")
          return left.model.sortNo - right.model.sortNo;
        const leftScore = Number(
          left.model.capabilities.includes("image_reference_consistency"),
        );
        const rightScore = Number(
          right.model.capabilities.includes("image_reference_consistency"),
        );
        return rightScore - leftScore || left.model.sortNo - right.model.sortNo;
      });
  }, [agent, modelIds, providers]);

  const save = useMutation({
    mutationFn: () => aiApi.writeAgentBinding(agent!.agentCode, modelIds),
    onSuccess: async () => {
      message.success(`${agent!.role} 的模型顺序已保存`);
      await onSaved();
    },
    onError: (error: Error) => message.error(error.message),
  });

  const move = (index: number, offset: number) => {
    const target = index + offset;
    if (target < 0 || target >= modelIds.length) return;
    const next = [...modelIds];
    [next[index], next[target]] = [next[target], next[index]];
    setModelIds(next);
  };

  return (
    <Modal
      title={agent ? `指定模型 · ${agent.role}` : "指定模型"}
      open={Boolean(agent)}
      onCancel={onClose}
      onOk={() => save.mutate()}
      okText="保存"
      okButtonProps={{ disabled: !canWrite, loading: save.isPending }}
      width={720}
      destroyOnHidden
    >
      <Space direction="vertical" className="workspace-main" size="middle">
        <Typography.Text type="secondary">
          仅显示已启用且支持 {agent?.requiredModelCapability ?? "所需能力"}{" "}
          的模型。顺序越靠前优先级越高。
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
                  <Button
                    key="up"
                    type="text"
                    icon={<ArrowUpOutlined />}
                    disabled={index === 0}
                    onClick={() => move(index, -1)}
                  />,
                  <Button
                    key="down"
                    type="text"
                    icon={<ArrowDownOutlined />}
                    disabled={index === modelIds.length - 1}
                    onClick={() => move(index, 1)}
                  />,
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
                    entry
                      ? `${entry.provider.name} · ${entry.model.modelId}`
                      : "模型已不存在，请移除后保存"
                  }
                />
              </List.Item>
            );
          }}
        />
        <Space.Compact block>
          <Select
            value={candidateId}
            placeholder="选择兼容模型"
            style={{ width: "100%" }}
            options={candidates.map(({ provider, model }) => ({
              value: model.id,
              label: `${provider.name} · ${model.displayName} (${model.modelId})`,
            }))}
            onChange={setCandidateId}
          />
          <Button
            icon={<PlusOutlined />}
            disabled={!candidateId}
            onClick={() => {
              if (!candidateId) return;
              setModelIds((current) => [...current, candidateId]);
              setCandidateId(undefined);
            }}
          >
            添加
          </Button>
        </Space.Compact>
      </Space>
    </Modal>
  );
}

export function modelStatus(model: AiModel, provider: AiProvider) {
  if (!provider.enabled) return "Provider 已停用";
  if (!model.enabled) return "模型已停用";
  if (!provider.hasKey) return "缺少 API Key";
  return "可用";
}
