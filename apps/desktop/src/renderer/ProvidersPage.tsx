import {
  ApiOutlined,
  DeleteOutlined,
  EditOutlined,
  ExportOutlined,
  ImportOutlined,
  PlusOutlined,
} from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Alert,
  App,
  Button,
  Card,
  Checkbox,
  Collapse,
  Form,
  Input,
  InputNumber,
  Modal,
  Popconfirm,
  Select,
  Space,
  Switch,
  Table,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { useMemo, useState } from "react";
import { aiApi } from "./api";
import { useStudio } from "./AppShell";
import type {
  AiAgentBinding,
  AiLimit,
  AiModel,
  AiProvider,
  ProviderPreset,
  ProviderPresetModel,
} from "./types";

const CUSTOM_PRESET = "__custom__";
const unsupportedDrivers = new Set([
  "ark_image",
  "ark_video",
  "dashscope_mm",
  "dashscope_async",
  "meshy",
]);
const capabilityOptions = [
  "text_reasoning",
  "text_structured_output",
  "vision_analysis",
  "image_text_to_image",
  "image_image_to_image",
  "image_reference_consistency",
  "video_text_to_video",
  "video_image_to_video",
  "model3d",
].map((value) => ({ value, label: value }));
const limitKinds = [
  "calls",
  "input_tokens",
  "output_tokens",
  "total_tokens",
  "tokens",
  "credits",
];
const limitUnits: Record<string, string> = {
  calls: "次",
  tokens: "tokens",
  credits: "credits",
};

type PresetModelValues = ProviderPresetModel & {
  id: string;
  selected: boolean;
  maxValue?: number | null;
  periodExpr: string;
  agentCodes: string[];
};

type ProviderValues = Pick<
  AiProvider,
  | "code"
  | "name"
  | "baseUrl"
  | "driver"
  | "authStyle"
  | "priority"
  | "enabled"
  | "remark"
> & {
  presetCode?: string;
  apiKey?: string;
  presetModels?: PresetModelValues[];
};

type ModelValues = Omit<AiModel, "id" | "providerCode" | "limits"> & {
  id?: string;
  limits?: AiLimit[];
};

export default function ProvidersPage() {
  const { message } = App.useApp();
  const { canWrite } = useStudio();
  const queryClient = useQueryClient();
  const [providerForm] = Form.useForm<ProviderValues>();
  const [modelForm] = Form.useForm<ModelValues>();
  const [providerEditing, setProviderEditing] = useState<AiProvider>();
  const [providerOpen, setProviderOpen] = useState(false);
  const [modelEditing, setModelEditing] = useState<AiModel>();
  const [modelProvider, setModelProvider] = useState<AiProvider>();
  const [modelOpen, setModelOpen] = useState(false);
  const [transferMode, setTransferMode] = useState<"export" | "import">();
  const [transferText, setTransferText] = useState("");
  const [importPreview, setImportPreview] = useState<{
    providerCount: number;
    modelCount: number;
  }>();

  const providers = useQuery({
    queryKey: ["ai-providers"],
    queryFn: aiApi.listProviders,
  });
  const presets = useQuery({
    queryKey: ["provider-presets"],
    queryFn: aiApi.listProviderPresets,
    staleTime: Infinity,
  });
  const agents = useQuery({
    queryKey: ["ai-agents"],
    queryFn: aiApi.listAgents,
    staleTime: Infinity,
  });
  const selectedPresetCode = Form.useWatch("presetCode", providerForm);
  const presetModels = Form.useWatch("presetModels", providerForm) ?? [];
  const selectedPreset = presets.data?.presets.find(
    (preset) => preset.code === selectedPresetCode,
  );
  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: ["ai-providers"] });

  const saveProvider = useMutation({
    mutationFn: async (values: ProviderValues) => {
      const code = values.code.trim();
      const models: AiModel[] = providerEditing
        ? providerEditing.models
        : (values.presetModels ?? [])
            .filter((model) => model.selected)
            .map((model, sortNo) => ({
              id: model.id,
              providerCode: code,
              modelId: model.modelId,
              displayName: model.modelId,
              capabilities: model.capabilities,
              driver: model.driver,
              apiPath: model.apiPath,
              enabled: true,
              sortNo,
              paramsJson: model.paramsJson,
              remark: model.remark,
              limits:
                model.maxValue && model.maxValue > 0
                  ? [
                      {
                        limitKind: model.limitKind,
                        maxValue: model.maxValue,
                        periodExpr: model.periodExpr,
                        groupName: "default",
                      },
                    ]
                  : [],
            }));
      const provider: AiProvider = {
        code,
        name: values.name.trim(),
        baseUrl: values.baseUrl.trim(),
        driver: values.driver.trim(),
        authStyle: values.authStyle,
        priority: values.priority ?? 0,
        enabled: values.enabled ?? true,
        remark: values.remark?.trim() ?? "",
        hasKey: providerEditing?.hasKey ?? false,
        keyMask: providerEditing?.keyMask ?? null,
        models,
      };
      if (providerEditing) {
        return aiApi.updateProvider(provider, values.apiKey || undefined);
      }
      const selectedIds = new Set(models.map((model) => model.id));
      const bindingMap = new Map<string, string[]>();
      for (const model of values.presetModels ?? []) {
        if (!selectedIds.has(model.id)) continue;
        for (const agentCode of model.agentCodes) {
          bindingMap.set(agentCode, [
            ...(bindingMap.get(agentCode) ?? []),
            model.id,
          ]);
        }
      }
      const agentBindings: AiAgentBinding[] = [...bindingMap].map(
        ([agentCode, modelIds]) => ({ agentCode, modelIds }),
      );
      return aiApi.createProvider(
        provider,
        values.apiKey || undefined,
        agentBindings,
      );
    },
    onSuccess: async () => {
      message.success(
        providerEditing ? "Provider 已更新" : "Provider 套餐已创建",
      );
      setProviderOpen(false);
      await refresh();
    },
    onError: (error: Error) => message.error(error.message),
  });

  const saveModel = useMutation({
    mutationFn: async (values: ModelValues) => {
      if (!modelProvider) throw new Error("未选择 Provider");
      const model: AiModel = {
        id: values.id || crypto.randomUUID(),
        providerCode: modelProvider.code,
        modelId: values.modelId.trim(),
        displayName: values.displayName.trim(),
        capabilities: values.capabilities,
        driver: values.driver.trim(),
        apiPath: values.apiPath.trim(),
        enabled: values.enabled ?? true,
        sortNo: values.sortNo ?? 0,
        paramsJson: values.paramsJson || "{}",
        remark: values.remark || "",
        limits: values.limits ?? [],
      };
      return aiApi.writeModel(modelEditing ? "update" : "create", model);
    },
    onSuccess: async () => {
      message.success("模型已保存");
      setModelOpen(false);
      await refresh();
    },
    onError: (error: Error) => message.error(error.message),
  });

  const removeProvider = useMutation({
    mutationFn: aiApi.deleteProvider,
    onSuccess: refresh,
    onError: (error: Error) => message.error(error.message),
  });
  const removeModel = useMutation({
    mutationFn: aiApi.deleteModel,
    onSuccess: refresh,
    onError: (error: Error) => message.error(error.message),
  });
  const toggleProvider = useMutation({
    mutationFn: (provider: AiProvider) =>
      aiApi.updateProvider({ ...provider, enabled: !provider.enabled }),
    onSuccess: refresh,
    onError: (error: Error) => message.error(error.message),
  });
  const toggleModel = useMutation({
    mutationFn: (model: AiModel) =>
      aiApi.writeModel("update", { ...model, enabled: !model.enabled }),
    onSuccess: refresh,
    onError: (error: Error) => message.error(error.message),
  });

  const openProvider = (provider?: AiProvider) => {
    setProviderEditing(provider);
    providerForm.setFieldsValue(
      provider
        ? {
            presetCode: CUSTOM_PRESET,
            code: provider.code,
            name: provider.name,
            baseUrl: provider.baseUrl,
            driver: provider.driver,
            authStyle: provider.authStyle,
            priority: provider.priority,
            enabled: provider.enabled,
            remark: provider.remark,
            apiKey: undefined,
            presetModels: [],
          }
        : {
            presetCode: undefined,
            code: "",
            name: "",
            baseUrl: "",
            driver: "openai_compat",
            authStyle: "bearer",
            priority: providers.data?.length ?? 0,
            enabled: true,
            remark: "",
            apiKey: undefined,
            presetModels: [],
          },
    );
    setProviderOpen(true);
  };

  const applyPreset = (code: string) => {
    if (code === CUSTOM_PRESET) {
      providerForm.setFieldsValue({
        presetCode: code,
        code: "",
        name: "",
        baseUrl: "",
        driver: "openai_compat",
        authStyle: "bearer",
        remark: "",
        presetModels: [],
      });
      return;
    }
    const preset = presets.data?.presets.find((item) => item.code === code);
    if (!preset) return;
    providerForm.setFieldsValue({
      presetCode: code,
      code: preset.code,
      name: preset.label,
      baseUrl: preset.baseUrl,
      driver: preset.driver,
      authStyle: preset.authStyle,
      remark: "",
      presetModels: preset.models.map((model) => ({
        ...model,
        id: crypto.randomUUID(),
        selected: true,
        maxValue: null,
        periodExpr: model.defaultPeriod,
        agentCodes: (agents.data ?? [])
          .filter((agent) =>
            model.capabilities.includes(agent.requiredModelCapability),
          )
          .map((agent) => agent.agentCode),
      })),
    });
  };

  const openModel = (provider: AiProvider, model?: AiModel) => {
    setModelProvider(provider);
    setModelEditing(model);
    modelForm.setFieldsValue(
      model ?? {
        modelId: "",
        displayName: "",
        capabilities: ["text_reasoning"],
        driver: provider.driver,
        apiPath: "/v1/responses",
        enabled: true,
        sortNo: provider.models.length,
        paramsJson: "{}",
        remark: "",
        limits: [],
      },
    );
    setModelOpen(true);
  };

  const exportConfig = async () => {
    setTransferMode("export");
    setTransferText(await aiApi.exportConfig());
  };
  const previewImport = async () => {
    const preview = await aiApi.importConfig(transferText, true);
    setImportPreview(preview);
  };
  const applyImport = async () => {
    await aiApi.importConfig(transferText, false);
    message.success("配置已导入，已保留仍存在 Provider 的 API Key");
    setTransferMode(undefined);
    setImportPreview(undefined);
    await refresh();
  };

  const modelColumns: ColumnsType<AiModel> = useMemo(
    () => [
      {
        title: "模型",
        render: (_, model) => (
          <Space direction="vertical" size={0}>
            <Typography.Text strong>{model.displayName}</Typography.Text>
            <Typography.Text type="secondary">{model.modelId}</Typography.Text>
            {unsupportedDrivers.has(model.driver) && (
              <Tag color="warning">仅配置，执行器待接入</Tag>
            )}
          </Space>
        ),
      },
      {
        title: "能力",
        render: (_, model) => (
          <Space wrap>
            {model.capabilities.map((item) => (
              <Tag key={item}>{item}</Tag>
            ))}
          </Space>
        ),
      },
      {
        title: "限额",
        render: (_, model) =>
          model.limits.length ? `${model.limits.length} 项` : "不限量",
      },
      {
        title: "启用",
        width: 72,
        render: (_, model) => (
          <Switch
            size="small"
            checked={model.enabled}
            disabled={!canWrite}
            onChange={() => toggleModel.mutate(model)}
          />
        ),
      },
      {
        title: "操作",
        width: 120,
        render: (_, model) => (
          <Space>
            <Button
              type="text"
              icon={<EditOutlined />}
              disabled={!canWrite}
              onClick={() =>
                openModel(
                  providers.data!.find(
                    (item) => item.code === model.providerCode,
                  )!,
                  model,
                )
              }
            />
            <Popconfirm
              title="删除该模型？"
              onConfirm={() => removeModel.mutate(model.id)}
            >
              <Button
                type="text"
                danger
                icon={<DeleteOutlined />}
                disabled={!canWrite}
              />
            </Popconfirm>
          </Space>
        ),
      },
    ],
    [canWrite, providers.data],
  );

  return (
    <div className="page-stack">
      <section className="page-heading">
        <div>
          <Typography.Title level={2}>Provider 与模型</Typography.Title>
          <Typography.Text type="secondary">
            从公共套餐预置创建账号；私有配置与 API Key 仅保存在
            .codex-game/local。
          </Typography.Text>
        </div>
        <Space wrap>
          <Button icon={<ExportOutlined />} onClick={() => void exportConfig()}>
            导出配置
          </Button>
          <Button
            icon={<ImportOutlined />}
            disabled={!canWrite}
            onClick={() => {
              setTransferMode("import");
              setTransferText("");
            }}
          >
            导入配置
          </Button>
          <Button
            type="primary"
            icon={<PlusOutlined />}
            disabled={!canWrite}
            onClick={() => openProvider()}
          >
            新建 Provider
          </Button>
        </Space>
      </section>

      {presets.error && (
        <Alert
          type="error"
          showIcon
          message="公共套餐文件校验失败"
          description={(presets.error as Error).message}
        />
      )}
      <Space direction="vertical" size="middle" className="workspace-main">
        {(providers.data ?? []).map((provider) => (
          <Card
            key={provider.code}
            className="content-card"
            title={
              <Space>
                <ApiOutlined />
                <span>{provider.name}</span>
                <Tag>{provider.code}</Tag>
                {provider.hasKey ? (
                  <Tag color="success">Key {provider.keyMask}</Tag>
                ) : (
                  <Tag color="warning">未配置 Key</Tag>
                )}
              </Space>
            }
            extra={
              <Space>
                <Switch
                  checked={provider.enabled}
                  disabled={!canWrite}
                  onChange={() => toggleProvider.mutate(provider)}
                />
                <Button
                  icon={<PlusOutlined />}
                  disabled={!canWrite}
                  onClick={() => openModel(provider)}
                >
                  添加模型
                </Button>
                <Button
                  icon={<EditOutlined />}
                  disabled={!canWrite}
                  onClick={() => openProvider(provider)}
                >
                  编辑
                </Button>
                <Popconfirm
                  title="删除 Provider 及其模型？"
                  onConfirm={() => removeProvider.mutate(provider.code)}
                >
                  <Button danger icon={<DeleteOutlined />} disabled={!canWrite}>
                    删除
                  </Button>
                </Popconfirm>
              </Space>
            }
          >
            <Typography.Paragraph type="secondary">
              {provider.driver} · {provider.authStyle} · 优先级{" "}
              {provider.priority} · {provider.baseUrl || "默认 Base URL"}
            </Typography.Paragraph>
            {provider.remark && (
              <Typography.Paragraph>{provider.remark}</Typography.Paragraph>
            )}
            <Table
              rowKey="id"
              size="small"
              pagination={false}
              dataSource={provider.models}
              columns={modelColumns}
              locale={{ emptyText: "尚未配置模型" }}
            />
          </Card>
        ))}
        {!providers.isLoading && !providers.data?.length && (
          <Card>
            <Typography.Text type="secondary">
              尚未配置 Provider，请从套餐预置创建账号。
            </Typography.Text>
          </Card>
        )}
      </Space>

      <Modal
        width={providerEditing ? 560 : 1080}
        title={providerEditing ? "编辑 Provider" : "从套餐新建 Provider"}
        open={providerOpen}
        confirmLoading={saveProvider.isPending}
        onCancel={() => setProviderOpen(false)}
        onOk={() => providerForm.submit()}
        destroyOnHidden
      >
        <Form
          form={providerForm}
          layout="vertical"
          onFinish={(values) => saveProvider.mutate(values)}
        >
          {!providerEditing && (
            <Form.Item
              name="presetCode"
              label="套餐"
              rules={[{ required: true, message: "请选择套餐或自定义" }]}
              extra="套餐会自动填入端点、Driver、鉴权方式与全部模型；同套餐多账号可修改 Code。"
            >
              <Select
                loading={presets.isLoading || agents.isLoading}
                onChange={applyPreset}
                options={[
                  ...(presets.data?.presets ?? []).map((preset) => ({
                    value: preset.code,
                    label: `${preset.label}（${preset.models.length} 个模型）`,
                  })),
                  { value: CUSTOM_PRESET, label: "自定义" },
                ]}
              />
            </Form.Item>
          )}
          {selectedPreset?.keyPrefix && (
            <Alert
              type="info"
              showIcon
              style={{ marginBottom: 16 }}
              message={`该套餐的 Key 以 ${selectedPreset.keyPrefix} 开头`}
              description="请确认使用对应套餐的 Key，避免请求未计入套餐额度。"
            />
          )}
          <Space className="form-row" align="start" wrap>
            <Form.Item
              name="code"
              label="Code"
              rules={[{ required: true }, { pattern: /^[A-Za-z0-9_-]+$/ }]}
            >
              <Input disabled={Boolean(providerEditing)} />
            </Form.Item>
            <Form.Item name="name" label="名称" rules={[{ required: true }]}>
              <Input />
            </Form.Item>
            <Form.Item name="priority" label="优先级">
              <InputNumber min={0} />
            </Form.Item>
            <Form.Item name="enabled" label="启用" valuePropName="checked">
              <Switch />
            </Form.Item>
          </Space>
          <Form.Item name="baseUrl" label="Base URL">
            <Input
              readOnly={Boolean(selectedPreset)}
              placeholder="https://api.example.com"
            />
          </Form.Item>
          <Space className="form-row" align="start" wrap>
            <Form.Item
              name="driver"
              label="Driver"
              rules={[{ required: true }]}
            >
              <Input readOnly={Boolean(selectedPreset)} />
            </Form.Item>
            <Form.Item
              name="authStyle"
              label="鉴权方式"
              rules={[{ required: true }]}
            >
              <Select
                disabled={Boolean(selectedPreset)}
                options={[
                  { value: "bearer", label: "Authorization: Bearer" },
                  { value: "x-api-key", label: "x-api-key" },
                ]}
              />
            </Form.Item>
          </Space>
          <Form.Item
            name="apiKey"
            label="API Key"
            extra={
              providerEditing?.hasKey
                ? `留空保持现有 Key（${providerEditing.keyMask}）；输入新值将替换。`
                : "可留空；明文只写入 ignored local secret 文件。"
            }
          >
            <Input.Password autoComplete="off" />
          </Form.Item>
          <Form.Item name="remark" label="账号备注">
            <Input.TextArea rows={2} />
          </Form.Item>

          {!providerEditing && selectedPreset && (
            <>
              <Typography.Title level={5}>模型与额度</Typography.Title>
              <Typography.Paragraph type="secondary">
                模型默认全选；额度留空表示不创建限制记录。能力匹配的 Agent
                会自动绑定。
              </Typography.Paragraph>
              <Form.List name="presetModels">
                {(fields) => (
                  <Space direction="vertical" className="workspace-main">
                    {fields.map((field, index) => {
                      const row = presetModels[index];
                      if (!row) return null;
                      return (
                        <Card key={field.key} size="small">
                          <Space align="start" wrap>
                            <Form.Item
                              name={[field.name, "selected"]}
                              valuePropName="checked"
                              style={{ marginBottom: 0 }}
                            >
                              <Checkbox />
                            </Form.Item>
                            <Space
                              direction="vertical"
                              size={2}
                              style={{ width: 350 }}
                            >
                              <Tooltip title={row.remark}>
                                <Typography.Text strong>
                                  {row.modelId}
                                </Typography.Text>
                              </Tooltip>
                              <Typography.Text type="secondary">
                                {row.driver} · {row.apiPath}
                              </Typography.Text>
                              <Typography.Text type="secondary">
                                {row.remark}
                              </Typography.Text>
                              <Space wrap size={2}>
                                {row.capabilities.map((capability) => (
                                  <Tag key={capability}>{capability}</Tag>
                                ))}
                                {contextWindow(row.paramsJson) && (
                                  <Tag>
                                    context {contextWindow(row.paramsJson)}
                                  </Tag>
                                )}
                                {unsupportedDrivers.has(row.driver) && (
                                  <Tag color="warning">
                                    仅配置，执行器待接入
                                  </Tag>
                                )}
                              </Space>
                              {row.agentCodes.length > 0 && (
                                <Typography.Text type="secondary">
                                  自动绑定：{row.agentCodes.join("、")}
                                </Typography.Text>
                              )}
                            </Space>
                            <Form.Item
                              name={[field.name, "maxValue"]}
                              style={{ marginBottom: 0 }}
                            >
                              <InputNumber
                                min={0}
                                placeholder="不限量"
                                addonAfter={
                                  limitUnits[row.limitKind] ?? row.limitKind
                                }
                              />
                            </Form.Item>
                            <Form.Item
                              name={[field.name, "periodExpr"]}
                              style={{ marginBottom: 0 }}
                              rules={[{ required: true }]}
                            >
                              <Input style={{ width: 130 }} />
                            </Form.Item>
                          </Space>
                        </Card>
                      );
                    })}
                  </Space>
                )}
              </Form.List>
            </>
          )}
        </Form>
      </Modal>

      <Modal
        width={720}
        title={modelEditing ? "编辑模型" : "添加模型"}
        open={modelOpen}
        confirmLoading={saveModel.isPending}
        onCancel={() => setModelOpen(false)}
        onOk={() => modelForm.submit()}
      >
        <Form
          form={modelForm}
          layout="vertical"
          onFinish={(values) => saveModel.mutate(values)}
        >
          <Form.Item name="id" hidden>
            <Input />
          </Form.Item>
          <Space className="form-row" align="start">
            <Form.Item
              name="modelId"
              label="模型 ID"
              rules={[{ required: true }]}
            >
              <Input />
            </Form.Item>
            <Form.Item
              name="displayName"
              label="展示名"
              rules={[{ required: true }]}
            >
              <Input />
            </Form.Item>
          </Space>
          <Form.Item
            name="capabilities"
            label="能力"
            rules={[{ required: true }]}
          >
            <Select mode="multiple" options={capabilityOptions} />
          </Form.Item>
          <Space className="form-row" align="start">
            <Form.Item
              name="driver"
              label="Driver"
              rules={[{ required: true }]}
            >
              <Input />
            </Form.Item>
            <Form.Item name="apiPath" label="API Path">
              <Input />
            </Form.Item>
            <Form.Item name="sortNo" label="排序">
              <InputNumber />
            </Form.Item>
            <Form.Item name="enabled" label="启用" valuePropName="checked">
              <Switch />
            </Form.Item>
          </Space>
          <Form.Item
            name="paramsJson"
            label="参数 JSON"
            rules={[
              {
                validator: (_, value) => {
                  try {
                    JSON.parse(value || "{}");
                    return Promise.resolve();
                  } catch {
                    return Promise.reject(new Error("请输入合法 JSON"));
                  }
                },
              },
            ]}
          >
            <Input.TextArea rows={3} />
          </Form.Item>
          <Form.Item name="remark" label="备注">
            <Input />
          </Form.Item>
          <Collapse
            items={[
              { key: "limits", label: "模型限额", children: <LimitFields /> },
            ]}
          />
        </Form>
      </Modal>

      <Modal
        width={720}
        title={transferMode === "export" ? "导出无密钥配置" : "导入无密钥配置"}
        open={Boolean(transferMode)}
        onCancel={() => {
          setTransferMode(undefined);
          setImportPreview(undefined);
        }}
        footer={
          transferMode === "import" ? (
            <Space>
              <Button onClick={() => setTransferMode(undefined)}>取消</Button>
              <Button
                onClick={() =>
                  void previewImport().catch((error: Error) =>
                    message.error(error.message),
                  )
                }
              >
                校验预览
              </Button>
              <Button
                type="primary"
                disabled={!importPreview}
                onClick={() =>
                  void applyImport().catch((error: Error) =>
                    message.error(error.message),
                  )
                }
              >
                确认覆盖
              </Button>
            </Space>
          ) : (
            <Button onClick={() => setTransferMode(undefined)}>关闭</Button>
          )
        }
      >
        <Alert type="warning" showIcon message="导入导出不包含任何 API Key" />
        {importPreview && (
          <Alert
            className="modal-alert"
            type="success"
            showIcon
            message={`校验通过：${importPreview.providerCount} 个 Provider，${importPreview.modelCount} 个模型`}
          />
        )}
        <Input.TextArea
          className="config-json"
          rows={18}
          readOnly={transferMode === "export"}
          value={transferText}
          onChange={(event) => {
            setTransferText(event.target.value);
            setImportPreview(undefined);
          }}
        />
      </Modal>
    </div>
  );
}

function contextWindow(paramsJson: string): string | undefined {
  try {
    const value = JSON.parse(paramsJson) as { context_window?: unknown };
    return typeof value.context_window === "number"
      ? value.context_window.toLocaleString()
      : undefined;
  } catch {
    return undefined;
  }
}

function LimitFields() {
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
                  placeholder="口径"
                  options={limitKinds.map((value) => ({ value, label: value }))}
                  style={{ width: 150 }}
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
                <Input placeholder="例如 day+11H" />
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
            增加限额
          </Button>
        </Space>
      )}
    </Form.List>
  );
}
