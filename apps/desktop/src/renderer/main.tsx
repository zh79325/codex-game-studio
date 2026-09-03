import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App as AntApp, ConfigProvider, theme } from "antd";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { HashRouter, Navigate, Route, Routes } from "react-router-dom";
import AgentsPage from "./AgentsPage";
import AppShell from "./AppShell";
import ProjectWorkspacePage from "./ProjectWorkspacePage";
import ProjectsPage from "./ProjectsPage";
import ProvidersPage from "./ProvidersPage";
import UsagePage from "./UsagePage";
import "./styles.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, staleTime: 1_000 },
    mutations: { retry: false },
  },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ConfigProvider
      theme={{
        algorithm: theme.darkAlgorithm,
        token: {
          colorPrimary: "#70d6a4",
          colorBgBase: "#0e1114",
          borderRadius: 8,
          fontFamily: "Inter, ui-sans-serif, system-ui, sans-serif",
        },
      }}
    >
      <AntApp>
        <QueryClientProvider client={queryClient}>
          <HashRouter>
            <Routes>
              <Route element={<AppShell />}>
                <Route index element={<Navigate to="/projects" replace />} />
                <Route path="/projects" element={<ProjectsPage />} />
                <Route
                  path="/projects/:projectId/workspace"
                  element={<ProjectWorkspacePage />}
                />
                <Route path="/ai/providers" element={<ProvidersPage />} />
                <Route path="/ai/agents" element={<AgentsPage />} />
                <Route path="/ai/usage" element={<UsagePage />} />
                <Route path="*" element={<Navigate to="/projects" replace />} />
              </Route>
            </Routes>
          </HashRouter>
        </QueryClientProvider>
      </AntApp>
    </ConfigProvider>
  </StrictMode>,
);
