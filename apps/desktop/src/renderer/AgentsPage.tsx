import {
  ArrowDownOutlined,
  ArrowUpOutlined,
  DeleteOutlined,
  PlusOutlined,
  RobotOutlined,
  SaveOutlined,
} from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Card,
  Col,
  Empty,
  List,
  Row,
  Select,
  Space,
  Switch,
  Tag,
  Typography,
} from "antd";
import { useEffect, useMemo, useState } from "react";
import { aiApi } from "./api";
import { useStudio } from "./AppShell";
import type { AiAgent, AiModel, AiProvider } from "./types";

type AgentEditorProps = {
  agent: AiAgent;
  providers: AiProvider[];
  canWrite: boolean;
  onSaved: () => Promise<unknown>;
};

export default function AgentsPage() {
  const { canWrite } = useStudio();
  const agents = useQuery({ queryKey: ["ai-agents"], queryFn: aiApi.listAgents });
  const providers = useQuery({ queryKey: ["ai-providers"], queryFn: aiApi.listProviders });
  const queryClient = useQueryClient();

  return (
    <div className="page-stack">
      <section className="page-heading">
        <div>
          <Typography.Title level={2}>Agent 配置</Typography.Title>
          <Typography.Text type="secondary">
            Agent 定义来自 bundled markdown；每个 Agent 只使用显式配置的有序模型短名单。
          </Typography.Text>
        </div>
      </section>
      <Row gutter={[16, 16]}>
        {(agents.data ?? []).map((agent) => (
          <Col xs={24} xl={12} key={agent.agentCode}>
            <AgentEditor
              agent={agent}
              providers={providers.data ?? []}
              canWrite={canWrite}
              onSaved={() => queryClient.invalidateQueries({ queryKey: ["ai-agents"] })}
            />
          </Col>
        ))}
      </Row>
      {!agents.isLoading && !agents.data?.length && <Card><Empty description="没有可配置的 bundled Agent" /></Card>}
    </div>
  );
}

function AgentEditor({ agent, providers, canWrite, onSaved }: AgentEditorProps) {
  const { message } = App.useApp();
  const [modelIds, setModelIds] = useState(agent.modelIds);
  const [providerCode, setProviderCode] = useState<string>();
  const [candidateId, setCandidateId] = useState<string>();
  const [showMismatch, setShowMismatch] = useState(false);

  useEffect(() => setModelIds(agent.modelIds), [agent.modelIds]);

  const modelMap = useMemo(
    () => new Map(providers.flatMap((provider) => provider.models.map((model) => [model.id, { provider, model }] as const))),
    [providers],
  );
  const selectedProvider = providers.find((provider) => provider.code === providerCode);
  const candidates = (selectedProvider?.models ?? []).filter(
    (model) => !modelIds.includes(model.id) && (showMismatch || model.capabilities.includes(agent.capability)),
  );

  const save = useMutation({
    mutationFn: () => aiApi.writeAgentBinding(agent.agentCode, modelIds),
    onSuccess: async () => {
      message.success(`${agent.role} 的模型顺序已保存`);
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
  const add = () => {
    if (!candidateId) return;
    setModelIds((current) => [...current, candidateId]);
    setCandidateId(undefined);
  };

  return (
    <Card
      className="content-card agent-card"
      title={<Space><RobotOutlined /><span>{agent.role}</span><Tag>{agent.agentCode}</Tag></Space>}
      extra={<Button type="primary" icon={<SaveOutlined />} disabled={!canWrite} loading={save.isPending} onClick={() => save.mutate()}>保存</Button>}
    >
      <Space direction="vertical" className="workspace-main" size="middle">
        <div>
          <Typography.Text type="secondary">所需能力</Typography.Text>
          <div><Tag color="blue">{agent.capability}</Tag><Tag>{agent.outputContract}</Tag></div>
          <Typography.Text type="secondary" className="path-text">{agent.sourceFile}</Typography.Text>
        </div>

        <div>
          <Typography.Text strong>有序回退列表</Typography.Text>
          <List
            className="binding-list"
            bordered
            locale={{ emptyText: "未绑定模型，运行时将明确报错" }}
            dataSource={modelIds}
            renderItem={(id, index) => {
              const entry = modelMap.get(id);
              return (
                <List.Item
                  actions={[
                    <Button key="up" type="text" icon={<ArrowUpOutlined />} disabled={!canWrite || index === 0} onClick={() => move(index, -1)} />,
                    <Button key="down" type="text" icon={<ArrowDownOutlined />} disabled={!canWrite || index === modelIds.length - 1} onClick={() => move(index, 1)} />,
                    <Button key="remove" type="text" danger icon={<DeleteOutlined />} disabled={!canWrite} onClick={() => setModelIds((current) => current.filter((item) => item !== id))} />,
                  ]}
                >
                  <List.Item.Meta
                    title={<Space><Tag color="geekblue">#{index + 1}</Tag><span>{entry?.model.displayName ?? id}</span></Space>}
                    description={entry ? `${entry.provider.name} · ${entry.model.modelId}` : "模型已不存在，请移除后保存"}
                  />
                </List.Item>
              );
            }}
          />
        </div>

        <div>
          <Space wrap align="end">
            <label className="field-stack">
              <Typography.Text type="secondary">第一步：Provider</Typography.Text>
              <Select
                value={providerCode}
                placeholder="选择 Provider"
                style={{ width: 190 }}
                options={providers.filter((provider) => provider.enabled).map((provider) => ({ value: provider.code, label: provider.name }))}
                onChange={(value) => { setProviderCode(value); setCandidateId(undefined); }}
              />
            </label>
            <label className="field-stack">
              <Typography.Text type="secondary">第二步：模型</Typography.Text>
              <Select
                value={candidateId}
                placeholder="选择模型"
                style={{ width: 260 }}
                disabled={!providerCode}
                options={candidates.map((model) => ({ value: model.id, label: `${model.displayName} (${model.modelId})` }))}
                onChange={setCandidateId}
              />
            </label>
            <Button icon={<PlusOutlined />} disabled={!canWrite || !candidateId} onClick={add}>加入回退列表</Button>
          </Space>
          <div className="mismatch-switch">
            <Space><Switch size="small" checked={showMismatch} onChange={setShowMismatch} /><Typography.Text type="secondary">查看能力不匹配模型</Typography.Text></Space>
          </div>
        </div>
      </Space>
    </Card>
  );
}

export function modelStatus(model: AiModel, provider: AiProvider) {
  if (!provider.enabled) return "Provider 已停用";
  if (!model.enabled) return "模型已停用";
  if (!provider.hasKey) return "缺少 API Key";
  return "可用";
}
