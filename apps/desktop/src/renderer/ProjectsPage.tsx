import {
  FolderAddOutlined,
  FolderOpenOutlined,
  ImportOutlined,
  PlayCircleOutlined,
} from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Card,
  Form,
  Input,
  Modal,
  Space,
  Table,
  Tag,
  Typography,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { projectsApi } from "./api";
import { useStudio } from "./AppShell";
import type { Project } from "./types";

type CreateValues = { name: string; root: string };
type ImportValues = { source: string; parent: string; folderName: string };

export default function ProjectsPage() {
  const { message } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { backend, setActiveProject } = useStudio();
  const [createOpen, setCreateOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [createForm] = Form.useForm<CreateValues>();
  const [importForm] = Form.useForm<ImportValues>();
  const canManage = backend.type === "ready" || backend.type === "readOnly";

  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: projectsApi.list,
    enabled: canManage,
  });

  const activate = async (project: Project) => {
    const opened = await projectsApi.open(project.root);
    setActiveProject(opened);
    navigate(`/projects/${opened.id}/workspace`);
  };

  const createProject = useMutation({
    mutationFn: ({ name, root }: CreateValues) => projectsApi.create(name, root),
    onSuccess: async (project) => {
      message.success("项目已创建");
      setCreateOpen(false);
      createForm.resetFields();
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
      setActiveProject(project);
      navigate(`/projects/${project.id}/workspace`);
    },
    onError: (error: Error) => message.error(error.message),
  });

  const openProject = useMutation({
    mutationFn: async () => {
      const root = await window.codexGame.selectDirectory("选择项目目录");
      return root ? projectsApi.open(root) : undefined;
    },
    onSuccess: async (project) => {
      if (!project) return;
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
      setActiveProject(project);
      navigate(`/projects/${project.id}/workspace`);
    },
    onError: (error: Error) => message.error(error.message),
  });

  const importProject = useMutation({
    mutationFn: async ({ source, parent, folderName }: ImportValues) => {
      const destination = `${parent.replace(/[\\/]$/, "")}/${folderName}`;
      return projectsApi.import(source, destination);
    },
    onSuccess: async ({ project, warnings }) => {
      setImportOpen(false);
      importForm.resetFields();
      if (warnings.length) message.warning(warnings.join("；"));
      else message.success("项目已导入");
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
      setActiveProject(project);
      navigate(`/projects/${project.id}/workspace`);
    },
    onError: (error: Error) => message.error(error.message),
  });

  const chooseDirectory = async (field: "root" | "source" | "parent") => {
    const selected = await window.codexGame.selectDirectory("选择目录");
    if (!selected) return;
    if (field === "root") createForm.setFieldValue(field, selected);
    else importForm.setFieldValue(field, selected);
  };

  const columns: ColumnsType<Project> = [
    {
      title: "项目",
      dataIndex: "name",
      render: (name: string, project) => (
        <Space direction="vertical" size={0}>
          <Typography.Text strong>{name}</Typography.Text>
          <Typography.Text type="secondary" className="path-text">
            {project.root}
          </Typography.Text>
        </Space>
      ),
    },
    {
      title: "状态",
      dataIndex: "state",
      width: 180,
      render: (state: string) => <Tag color="blue">{state}</Tag>,
    },
    {
      title: "操作",
      width: 120,
      render: (_, project) => (
        <Button
          type="primary"
          ghost
          icon={<PlayCircleOutlined />}
          onClick={() => void activate(project)}
        >
          进入
        </Button>
      ),
    },
  ];

  return (
    <div className="page-stack">
      <section className="page-heading">
        <div>
          <Typography.Title level={2}>项目</Typography.Title>
          <Typography.Text type="secondary">
            创建、导入并进入游戏项目。运行数据保存在项目内，不写入默认用户目录。
          </Typography.Text>
        </div>
        <Space>
          <Button
            icon={<FolderOpenOutlined />}
            loading={openProject.isPending}
            disabled={!canManage}
            onClick={() => openProject.mutate()}
          >
            打开项目
          </Button>
          <Button
            icon={<ImportOutlined />}
            disabled={!canManage}
            onClick={() => setImportOpen(true)}
          >
            导入旧项目
          </Button>
          <Button
            type="primary"
            icon={<FolderAddOutlined />}
            disabled={!canManage}
            onClick={() => setCreateOpen(true)}
          >
            新建项目
          </Button>
        </Space>
      </section>

      <Card className="content-card">
        <Table<Project>
          rowKey="id"
          loading={projects.isLoading}
          dataSource={projects.data ?? []}
          columns={columns}
          pagination={false}
          locale={{ emptyText: "还没有项目，请新建或打开一个项目" }}
        />
      </Card>

      <Modal
        title="新建项目"
        open={createOpen}
        okText="创建并进入"
        cancelText="取消"
        confirmLoading={createProject.isPending}
        onCancel={() => setCreateOpen(false)}
        onOk={() => createForm.submit()}
      >
        <Form form={createForm} layout="vertical" onFinish={(values) => createProject.mutate(values)}>
          <Form.Item name="name" label="项目名称" rules={[{ required: true }]}>
            <Input autoFocus placeholder="例如：星海旅人" />
          </Form.Item>
          <Form.Item label="项目目录" required>
            <Space.Compact block>
              <Form.Item name="root" noStyle rules={[{ required: true }]}>
                <Input readOnly placeholder="选择一个空目录" />
              </Form.Item>
              <Button onClick={() => void chooseDirectory("root")}>选择</Button>
            </Space.Compact>
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        title="导入旧项目"
        open={importOpen}
        okText="导入并进入"
        cancelText="取消"
        confirmLoading={importProject.isPending}
        onCancel={() => setImportOpen(false)}
        onOk={() => importForm.submit()}
      >
        <Form form={importForm} layout="vertical" onFinish={(values) => importProject.mutate(values)}>
          <DirectoryField form={importForm} name="source" label="旧项目目录" onChoose={chooseDirectory} />
          <DirectoryField form={importForm} name="parent" label="导入到" onChoose={chooseDirectory} />
          <Form.Item name="folderName" label="新目录名称" rules={[{ required: true }]}>
            <Input placeholder="game-project" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}

function DirectoryField({
  form,
  name,
  label,
  onChoose,
}: {
  form: ReturnType<typeof Form.useForm<ImportValues>>[0];
  name: "source" | "parent";
  label: string;
  onChoose: (field: "root" | "source" | "parent") => Promise<void>;
}) {
  return (
    <Form.Item label={label} required>
      <Space.Compact block>
        <Form.Item name={name} noStyle rules={[{ required: true }]}>
          <Input readOnly />
        </Form.Item>
        <Button onClick={() => void onChoose(name)}>选择</Button>
      </Space.Compact>
    </Form.Item>
  );
}
