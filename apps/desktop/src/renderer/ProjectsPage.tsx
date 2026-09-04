import {
  DeleteOutlined,
  FolderAddOutlined,
  FolderOpenOutlined,
  PlayCircleOutlined,
} from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { App, Button, Card, Modal, Space, Table, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { projectsApi } from "./api";
import { useStudio } from "./AppShell";
import type { Project } from "./types";

type ListedProject = Project & {
  projectFileExists: boolean;
};

export default function ProjectsPage() {
  const { message, modal } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { backend, setActiveProject } = useStudio();
  const [bootstrapOpen, setBootstrapOpen] = useState(false);
  const [root, setRoot] = useState<string>();
  const canManage = backend.type === "ready" || backend.type === "readOnly";

  const projects = useQuery<ListedProject[]>({
    queryKey: ["projects"],
    queryFn: async () => {
      const listed = await projectsApi.list();
      return Promise.all(
        listed.map(async (project) => {
          const inspected = await projectsApi.inspect(project.root);
          return {
            ...project,
            projectFileExists:
              inspected.supported && inspected.projectId === project.id,
          };
        }),
      );
    },
    enabled: canManage,
  });

  const enter = (project: Project) => {
    setActiveProject(project);
    navigate(`/projects/${project.id}/workspace`);
  };

  const activate = async (project: Project) =>
    enter(await projectsApi.open(project.root));

  const createProject = useMutation({
    mutationFn: async ({
      path,
      overwrite,
    }: {
      path: string;
      overwrite: boolean;
    }) => projectsApi.create(path, overwrite),
    onSuccess: async (project) => {
      message.success("素材项目已初始化，请先确认美术基调");
      setBootstrapOpen(false);
      setRoot(undefined);
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
      enter(project);
    },
    onError: (error: Error) => message.error(error.message),
  });

  const chooseBootstrapRoot = async () => {
    const selected = await window.codexGame.selectDirectory("选择素材项目目录");
    if (!selected) return;
    setRoot(selected);
  };

  const bootstrap = async () => {
    if (!root) return;
    try {
      const inspected = await projectsApi.inspect(root);
      if (!inspected.supported && inspected.occupied) {
        message.error("该目录包含不受支持的旧项目，请选择新目录或明确覆盖。");
      }
      if (inspected.occupied) {
        modal.confirm({
          title: "覆盖本应用项目数据？",
          content:
            "只会清理 project.json、art-bible.md 和本地运行库，不会删除用户素材。",
          okText: "安全覆盖",
          okButtonProps: { danger: true },
          onOk: () =>
            createProject.mutateAsync({ path: root, overwrite: true }),
        });
        return;
      }
      createProject.mutate({ path: root, overwrite: false });
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const removeProject = useMutation({
    mutationFn: projectsApi.remove,
    onSuccess: async () => {
      message.success("已从项目列表移除");
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
    onError: (error: Error) => message.error(error.message),
  });

  const confirmRemove = (project: ListedProject) => {
    modal.confirm({
      title: "移除失效项目？",
      content: `${project.root} 下缺少有效的 project.json。此操作只移除项目列表记录，不会删除目录中的其他文件。`,
      okText: "移除",
      okButtonProps: { danger: true },
      onOk: () => removeProject.mutateAsync(project.id),
    });
  };

  const openProject = useMutation({
    mutationFn: async () => {
      const selected = await window.codexGame.selectDirectory("选择项目目录");
      if (!selected) return undefined;
      const inspected = await projectsApi.inspect(selected);
      if (!inspected.supported)
        throw new Error("该项目版本不受支持，请新建项目。");
      return projectsApi.open(selected);
    },
    onSuccess: async (project) => {
      if (!project) return;
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
      enter(project);
    },
    onError: (error: Error) => message.error(error.message),
  });

  const columns: ColumnsType<ListedProject> = [
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
      render: (state: Project["state"], project) =>
        project.projectFileExists ? (
          <Tag color={state === "ready" ? "success" : "processing"}>
            {state}
          </Tag>
        ) : (
          <Tag color="error">项目文件不存在</Tag>
        ),
    },
    {
      title: "操作",
      width: 120,
      render: (_, project) =>
        project.projectFileExists ? (
          <Button
            type="primary"
            ghost
            icon={<PlayCircleOutlined />}
            onClick={() => void activate(project)}
          >
            进入
          </Button>
        ) : (
          <Button
            danger
            icon={<DeleteOutlined />}
            loading={
              removeProject.isPending && removeProject.variables === project.id
            }
            onClick={() => confirmRemove(project)}
          >
            移除
          </Button>
        ),
    },
  ];

  return (
    <div className="page-stack">
      <section className="page-heading">
        <div>
          <Typography.Title level={2}>素材项目</Typography.Title>
          <Typography.Text type="secondary">
            先选择目录，再通过 Action 会话确认 Art Bible 和项目名称。
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
            type="primary"
            icon={<FolderAddOutlined />}
            disabled={!canManage}
            onClick={() => setBootstrapOpen(true)}
          >
            新建项目
          </Button>
        </Space>
      </section>

      <Card className="content-card">
        <Table<ListedProject>
          rowKey="id"
          loading={projects.isLoading}
          dataSource={projects.data ?? []}
          columns={columns}
          pagination={false}
          locale={{ emptyText: "还没有素材项目" }}
        />
      </Card>

      <Modal
        title="初始化素材项目"
        open={bootstrapOpen}
        okText="创建并进入"
        confirmLoading={createProject.isPending}
        okButtonProps={{ disabled: !root }}
        onCancel={() => setBootstrapOpen(false)}
        onOk={() => void bootstrap()}
      >
        <Space direction="vertical" className="workspace-main">
          <Typography.Text type="secondary">
            此阶段只选择目录，项目名称和代号将在 Art Bible 确认后填写。
          </Typography.Text>
          <Button
            block
            icon={<FolderOpenOutlined />}
            onClick={() => void chooseBootstrapRoot()}
          >
            {root ?? "选择目录"}
          </Button>
        </Space>
      </Modal>
    </div>
  );
}
