import {
  ClearOutlined,
  ReloadOutlined,
  SafetyCertificateOutlined,
} from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Card,
  Popconfirm,
  Progress,
  Space,
  Table,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { aiApi } from "./api";
import { useStudio } from "./AppShell";
import type { AiModelUsage, UsageBudget } from "./types";

export default function UsagePage() {
  const { message } = App.useApp();
  const { canWrite } = useStudio();
  const queryClient = useQueryClient();
  const usage = useQuery({
    queryKey: ["ai-usage"],
    queryFn: aiApi.readUsage,
    refetchInterval: 10_000,
  });
  const refresh = () => queryClient.invalidateQueries({ queryKey: ["ai-usage"] });
  const reset = useMutation({
    mutationFn: ({ modelId, limitKind }: { modelId: string; limitKind?: string }) =>
      aiApi.resetUsage(modelId, limitKind),
    onSuccess: async (result) => {
      message.success(`已清理 ${result.cleared} 条本地用量记录`);
      await refresh();
    },
    onError: (error: Error) => message.error(error.message),
  });
  const clearBreaker = useMutation({
    mutationFn: aiApi.clearBreaker,
    onSuccess: async () => {
      message.success("熔断状态已解除");
      await refresh();
    },
    onError: (error: Error) => message.error(error.message),
  });

  const columns: ColumnsType<AiModelUsage> = [
    {
      title: "Provider / 模型",
      width: 250,
      render: (_, item) => (
        <Space direction="vertical" size={1}>
          <Space><Typography.Text strong>{item.providerName}</Typography.Text><Tag>{item.providerCode}</Tag></Space>
          <Typography.Text>{item.modelId}</Typography.Text>
          <Space wrap>
            {!item.providerEnabled || !item.enabled ? <Tag color="default">已停用</Tag> : <Tag color="success">已启用</Tag>}
            {item.hasKey ? <Tag color="blue">Key 已配置</Tag> : <Tag color="warning">缺少 Key</Tag>}
            <Tag color="cyan">本地账本</Tag>
          </Space>
        </Space>
      ),
    },
    {
      title: "Agent",
      width: 180,
      render: (_, item) => <Space wrap>{item.agents.length ? item.agents.map((agent) => <Tag key={agent}>{agent}</Tag>) : <Typography.Text type="secondary">未绑定</Typography.Text>}</Space>,
    },
    {
      title: "额度窗口",
      render: (_, item) => item.budgets.length ? <Space direction="vertical" className="budget-list">{item.budgets.map((budget) => <BudgetBar key={`${budget.limitKind}:${budget.groupName}`} budget={budget} onReset={() => reset.mutate({ modelId: item.providerModelId, limitKind: budget.limitKind })} disabled={!canWrite} />)}</Space> : <Typography.Text type="secondary">未设置限额</Typography.Text>,
    },
    {
      title: "熔断",
      width: 210,
      render: (_, item) => item.breaker ? (
        <Space direction="vertical" size={2}>
          <Tag color={item.breaker.openedAt ? "error" : "warning"}>连续失败 {item.breaker.failureCount}</Tag>
          {item.breaker.lastReason && <Tooltip title={item.breaker.lastReason}><Typography.Text ellipsis className="breaker-reason">{item.breaker.lastReason}</Typography.Text></Tooltip>}
          {item.breaker.retryAt && <Typography.Text type="secondary">恢复：{new Date(item.breaker.retryAt * 1000).toLocaleString()}</Typography.Text>}
          <Button size="small" icon={<SafetyCertificateOutlined />} disabled={!canWrite} loading={clearBreaker.isPending} onClick={() => clearBreaker.mutate(item.providerModelId)}>手动放行</Button>
        </Space>
      ) : <Tag color="success">正常</Tag>,
    },
    {
      title: "操作",
      width: 110,
      render: (_, item) => (
        <Popconfirm title="清理该模型全部本地用量？" onConfirm={() => reset.mutate({ modelId: item.providerModelId })}>
          <Button danger type="text" icon={<ClearOutlined />} disabled={!canWrite}>清理</Button>
        </Popconfirm>
      ),
    },
  ];

  return (
    <div className="page-stack">
      <section className="page-heading">
        <div>
          <Typography.Title level={2}>额度与用量</Typography.Title>
          <Typography.Text type="secondary">每 10 秒刷新本地 ledger。当前 Provider 驱动不支持远程对账，因此所有数据均标记为本地来源。</Typography.Text>
        </div>
        <Button icon={<ReloadOutlined />} loading={usage.isFetching} onClick={() => void usage.refetch()}>刷新</Button>
      </section>
      <Card className="content-card">
        <Table<AiModelUsage>
          rowKey="providerModelId"
          loading={usage.isLoading}
          dataSource={usage.data ?? []}
          columns={columns}
          pagination={false}
          scroll={{ x: 1080 }}
          locale={{ emptyText: "尚无模型用量数据" }}
        />
      </Card>
    </div>
  );
}

function BudgetBar({ budget, onReset, disabled }: { budget: UsageBudget; onReset: () => void; disabled: boolean }) {
  const percent = budget.unlimited ? 0 : Math.min(100, Math.round((budget.used / Math.max(1, budget.limit)) * 100));
  return (
    <div className="budget-row">
      <div className="budget-heading">
        <Space><Tag>{budget.limitKind}</Tag><Typography.Text type="secondary">{budget.groupName} · {budget.periodExpr}</Typography.Text></Space>
        <Space>
          <Typography.Text>{budget.used} / {budget.unlimited ? "∞" : budget.limit}</Typography.Text>
          <Button size="small" type="link" disabled={disabled} onClick={onReset}>重置</Button>
        </Space>
      </div>
      <Progress percent={percent} status={budget.exhausted ? "exception" : "active"} showInfo={!budget.unlimited} />
    </div>
  );
}
