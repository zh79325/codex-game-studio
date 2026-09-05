import {
  ApiOutlined,
  DashboardOutlined,
  FolderOpenOutlined,
  RobotOutlined,
  SettingOutlined,
} from "@ant-design/icons";
import { useQueryClient } from "@tanstack/react-query";
import { Badge, Layout, Menu, Space, Spin, Tag, Typography } from "antd";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Outlet,
  useLocation,
  useNavigate,
  useOutletContext,
} from "react-router-dom";
import type { BackendState } from "../shared/ipc";
import type { Project } from "./types";

const brandIconUrl = new URL("./brand-icon.svg", import.meta.url).href;

export type StudioContext = {
  backend: BackendState;
  canWrite: boolean;
  activeProject?: Project;
  setActiveProject: (project?: Project) => void;
};

export const useStudio = () => useOutletContext<StudioContext>();

export default function AppShell() {
  const navigate = useNavigate();
  const location = useLocation();
  const queryClient = useQueryClient();
  const [backend, setBackend] = useState<BackendState>({ type: "starting" });
  const [activeProject, setActiveProject] = useState<Project>();
  const previousBackendType = useRef(backend.type);

  useEffect(() => {
    const removeState = window.codexGame.onBackendState((state) => {
      const wasConnected = ["ready", "readOnly"].includes(
        previousBackendType.current,
      );
      const isConnected = ["ready", "readOnly"].includes(state.type);
      previousBackendType.current = state.type;
      setBackend(state);
      if (isConnected && !wasConnected) void queryClient.invalidateQueries();
    });
    const removeEvent = window.codexGame.onEvent((event) => {
      if (typeof event === "object" && event && "method" in event) {
        if (String(event.method).startsWith("game/")) {
          void queryClient.invalidateQueries();
        }
      }
    });
    return () => {
      removeState();
      removeEvent();
    };
  }, [queryClient]);

  const status = useMemo(() => {
    switch (backend.type) {
      case "ready":
        return { color: "success", text: `后端 ${backend.backendVersion}` };
      case "readOnly":
        return { color: "warning", text: `只读 ${backend.backendVersion}` };
      case "starting":
        return { color: "processing", text: "后端启动中" };
      case "recovering":
        return { color: "processing", text: "正在恢复" };
      case "incompatible":
      case "stopped":
        return { color: "error", text: backend.message };
    }
  }, [backend]);

  const overlay =
    backend.type === "starting"
      ? ["后端启动中", "正在启动项目内 codex-app-server，请稍候。"]
      : backend.type === "recovering"
        ? ["正在恢复", "后端恢复完成后会自动刷新当前页面。"]
        : backend.type === "incompatible" || backend.type === "stopped"
          ? ["后端不可用", backend.message]
          : undefined;

  const selectedKey = location.pathname.startsWith("/ai/providers")
    ? "providers"
    : location.pathname.startsWith("/ai/agents")
      ? "agents"
      : location.pathname.startsWith("/ai/usage")
        ? "usage"
        : location.pathname.includes("/workspace") ||
            location.pathname.includes("/characters/")
          ? "workspace"
          : "projects";

  return (
    <Layout className="app-shell">
      <Layout.Header className="app-header">
        <button className="brand" onClick={() => navigate("/projects")}>
          <img className="brand-mark" src={brandIconUrl} alt="" />
          <span>Codex Game Studio</span>
        </button>
        <Menu
          className="main-menu"
          mode="horizontal"
          theme="dark"
          selectedKeys={[selectedKey]}
          onClick={({ key }) => {
            if (key === "projects") navigate("/projects");
            if (key === "workspace" && activeProject)
              navigate(`/projects/${activeProject.id}/workspace`);
            if (key === "providers") navigate("/ai/providers");
            if (key === "agents") navigate("/ai/agents");
            if (key === "usage") navigate("/ai/usage");
          }}
          items={[
            { key: "projects", icon: <FolderOpenOutlined />, label: "项目" },
            {
              key: "workspace",
              icon: <DashboardOutlined />,
              label: activeProject?.name ?? "项目工作区",
              disabled: !activeProject,
            },
            {
              key: "ai",
              icon: <SettingOutlined />,
              label: "AI 配置",
              children: [
                {
                  key: "providers",
                  icon: <ApiOutlined />,
                  label: "Provider 与模型",
                },
                { key: "agents", icon: <RobotOutlined />, label: "Agent 配置" },
                {
                  key: "usage",
                  icon: <DashboardOutlined />,
                  label: "额度看板",
                },
              ],
            },
          ]}
        />
        <Space className="backend-status">
          <Badge status={status.color as "success"} />
          <Typography.Text>{status.text}</Typography.Text>
          {backend.type === "readOnly" && <Tag color="warning">只读</Tag>}
        </Space>
      </Layout.Header>
      <Layout.Content className="app-content">
        <Outlet
          context={{
            backend,
            canWrite: backend.type === "ready",
            activeProject,
            setActiveProject,
          }}
        />
      </Layout.Content>
      {overlay && (
        <div className="backend-overlay" data-state={backend.type}>
          <div className="backend-overlay-card">
            <Spin size="large" spinning={backend.type !== "stopped"} />
            <Typography.Title level={4}>{overlay[0]}</Typography.Title>
            <Typography.Text type="secondary">{overlay[1]}</Typography.Text>
          </div>
        </div>
      )}
    </Layout>
  );
}
