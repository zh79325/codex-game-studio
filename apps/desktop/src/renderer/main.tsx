import { StrictMode, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import type {
  GameArtBibleVersion,
  GameConflict,
  GameFocusWorkflow,
  GameReviewReport,
  GameTask,
  GameUserDecision,
} from "../generated/game";
import type { BackendState } from "../shared/ipc";
import "./styles.css";

type FocusSnapshot = {
  workflow: GameFocusWorkflow;
  reviews: GameReviewReport[];
  conflicts: GameConflict[];
  artBibleDraft: string | null;
  decisions: GameUserDecision[];
};

function App() {
  const [backend, setBackend] = useState<BackendState>({ type: "starting" });
  const [message, setMessage] = useState("");
  const [projectId, setProjectId] = useState<string>();
  const [conversationId, setConversationId] = useState<string>();
  const [focus, setFocus] = useState<FocusSnapshot>();
  const [tasks, setTasks] = useState<GameTask[]>([]);
  const [versions, setVersions] = useState<GameArtBibleVersion[]>([]);
  const [selectedVersion, setSelectedVersion] = useState<number>();
  const [artBible, setArtBible] = useState("");
  const [events, setEvents] = useState<unknown[]>([]);
  const previousBackendType = useRef(backend.type);

  useEffect(() => {
    const removeState = window.codexGame.onBackendState((state) => {
      const wasConnected =
        previousBackendType.current === "ready" ||
        previousBackendType.current === "readOnly";
      const isConnected = state.type === "ready" || state.type === "readOnly";
      previousBackendType.current = state.type;
      setBackend(state);
      // Crash-recovery handshake: once the backend returns to a connected
      // state, re-read the projection so the UI reflects durable state
      // instead of the snapshot captured before the restart.
      if (isConnected && !wasConnected) void refreshWorkspace();
    });
    const removeEvent = window.codexGame.onEvent((event) => {
      setEvents((current) => [event, ...current].slice(0, 100));
      if (typeof event !== "object" || event === null || !("method" in event))
        return;
      if (event.method === "game/designConfirmation/required") {
        void window.codexGame.notify(
          "需要设计确认",
          "Art Bible 草案包含高影响冲突，请返回应用完成决策。",
        );
      }
      if (String(event.method).startsWith("game/")) {
        void refreshWorkspace();
      }
    });
    return () => {
      removeState();
      removeEvent();
    };
  }, [conversationId, projectId]);

  const canSubmit = backend.type === "ready";
  const canManageProjects =
    backend.type === "ready" || backend.type === "readOnly";
  const statusText = useMemo(() => {
    switch (backend.type) {
      case "starting":
        return "后端启动中";
      case "recovering":
        return "正在恢复";
      case "ready":
        return `已连接 ${backend.backendVersion}`;
      case "readOnly":
        return `只读模式 ${backend.backendVersion}（提交已禁用）`;
      case "incompatible":
      case "stopped":
        return backend.message;
    }
  }, [backend]);

  const overlay = useMemo(() => {
    switch (backend.type) {
      case "starting":
        return {
          title: "后端启动中",
          detail: "正在启动 codex-app-server，请稍候。",
        };
      case "recovering":
        return {
          title: "正在恢复",
          detail: "后端正在恢复，完成后会自动继续，提交已暂时禁用。",
        };
      case "incompatible":
        return { title: "版本不兼容", detail: backend.message };
      case "stopped":
        return { title: "后端已停止", detail: backend.message };
      case "ready":
      case "readOnly":
        return undefined;
    }
  }, [backend]);

  async function refreshWorkspace(
    activeProjectId = projectId,
    activeConversationId = conversationId,
  ) {
    if (!activeProjectId || !activeConversationId) return;
    const [focusResult, taskResult, versionResult] = await Promise.all([
      window.codexGame.request<FocusSnapshot>("game/focus/read", {
        conversationId: activeConversationId,
      }),
      window.codexGame.request<{ tasks: GameTask[] }>("game/task/list", {
        conversationId: activeConversationId,
      }),
      window.codexGame.request<{ versions: GameArtBibleVersion[] }>(
        "game/artBible/list",
        {
          projectId: activeProjectId,
        },
      ),
    ]);
    setFocus(focusResult);
    setTasks(taskResult.tasks);
    setVersions(versionResult.versions);
  }

  async function activateProject(project: {
    id: string;
    name: string;
    root: string;
  }) {
    setProjectId(project.id);
    try {
      const ensured = await window.codexGame.request<{
        conversation: { id: string };
      }>("game/conversation/ensure", { projectId: project.id });
      await window.codexGame.request("game/focus/start", {
        conversationId: ensured.conversation.id,
      });
      setConversationId(ensured.conversation.id);
      await refreshWorkspace(project.id, ensured.conversation.id);
    } catch (error) {
      setConversationId(undefined);
      setFocus(undefined);
      setTasks([]);
      setVersions([]);
      setEvents((current) => [{ project, error: String(error) }, ...current]);
    }
  }

  async function decide(action: string, extras: Record<string, unknown> = {}) {
    if (!conversationId || !focus || !canSubmit) return;
    try {
      await window.codexGame.request("game/focus/decide", {
        conversationId,
        expectedInputVersion: Number(focus.workflow.inputVersion),
        action,
        ...extras,
      });
      await refreshWorkspace();
    } catch (error) {
      setEvents((current) => [{ error: String(error) }, ...current]);
    }
  }

  async function loadArtBible(version: number) {
    if (!projectId) return;
    try {
      const result = await window.codexGame.request<{ markdown: string }>(
        "game/artBible/read",
        { projectId, version },
      );
      setSelectedVersion(version);
      setArtBible(result.markdown);
    } catch (error) {
      setEvents((current) => [{ error: String(error) }, ...current]);
    }
  }

  async function createProject() {
    const name = window.prompt("项目名称");
    const root = await window.codexGame.selectDirectory("选择新项目目录");
    if (!name || !root) return;
    try {
      const result = await window.codexGame.request<{
        project: { id: string; name: string; root: string };
      }>("game/project/create", { name, root });
      await activateProject(result.project);
    } catch (error) {
      setEvents((current) => [{ error: String(error) }, ...current]);
    }
  }

  async function openProject() {
    const root = await window.codexGame.selectDirectory("选择项目目录");
    if (!root) return;
    try {
      const result = await window.codexGame.request<{
        project: { id: string; name: string; root: string };
      }>("game/project/open", { root });
      await activateProject(result.project);
    } catch (error) {
      setEvents((current) => [{ error: String(error) }, ...current]);
    }
  }

  async function importProject() {
    const source = await window.codexGame.selectDirectory("选择旧项目目录");
    const parent =
      await window.codexGame.selectDirectory("选择导入目标的父目录");
    const folderName = window.prompt("新项目目录名称");
    if (!source || !parent || !folderName) return;
    const destination = `${parent.replace(/[\\/]$/, "")}/${folderName}`;
    try {
      const result = await window.codexGame.request<{
        project: { id: string; name: string; root: string };
        warnings: string[];
      }>("game/project/import", { source, destination });
      await activateProject(result.project);
      setEvents((current) => [{ warnings: result.warnings }, ...current]);
    } catch (error) {
      setEvents((current) => [{ error: String(error) }, ...current]);
    }
  }

  async function submit() {
    const content = message.trim();
    if (!content || !canSubmit || !conversationId) return;
    setMessage("");
    try {
      const result = await window.codexGame.request(
        "game/conversation/submit",
        {
          conversationId,
          content,
        },
      );
      setEvents((current) => [result, ...current]);
      await refreshWorkspace();
    } catch (error) {
      setEvents((current) => [{ error: String(error) }, ...current]);
    }
  }

  return (
    <main>
      {overlay && (
        <div className="backend-overlay" data-state={backend.type}>
          <div className="backend-overlay-card">
            <h2>{overlay.title}</h2>
            <p>{overlay.detail}</p>
          </div>
        </div>
      )}
      <header>
        <h1>Codex Game Studio</h1>
        <span data-state={backend.type}>{statusText}</span>
      </header>
      <section className="workspace">
        <aside>
          <button
            onClick={() => void createProject()}
            disabled={!canManageProjects}
          >
            新建项目
          </button>
          <button
            onClick={() => void openProject()}
            disabled={!canManageProjects}
          >
            打开项目
          </button>
          <button
            onClick={() => void importProject()}
            disabled={!canManageProjects}
          >
            导入项目
          </button>
          {projectId && <p className="project-id">项目：{projectId}</p>}
          <h2>对焦流程</h2>
          <ol>
            {[
              ["游戏简报", ["CLARIFYING", "BRIEF_READY"]],
              ["并行评审", ["REVIEWING", "MERGING"]],
              ["冲突决策", ["USER_REVIEW"]],
              ["Art Bible", ["CONFIRMED", "VERSIONED"]],
            ].map(([label, states]) => (
              <li
                key={String(label)}
                className={
                  Array.isArray(states) &&
                  states.includes(focus?.workflow.state ?? "")
                    ? "active"
                    : undefined
                }
              >
                {label}
              </li>
            ))}
          </ol>
          <h2>Art Bible 版本</h2>
          <div className="version-list">
            {versions.map((version) => (
              <button
                key={version.id}
                className={
                  selectedVersion === Number(version.version)
                    ? "selected"
                    : undefined
                }
                onClick={() => void loadArtBible(Number(version.version))}
              >
                v{String(version.version)}
              </button>
            ))}
          </div>
        </aside>
        <section className="conversation">
          <div className="events">
            {!focus && <p>描述你的游戏创意，开始设定对焦。</p>}
            {focus && (
              <section className="focus-panel">
                <div className="panel-heading">
                  <h2>{focus.workflow.state}</h2>
                  <span>输入版本 {String(focus.workflow.inputVersion)}</span>
                </div>
                {focus.workflow.state === "BRIEF_READY" && (
                  <button
                    onClick={() => void decide("acceptBrief")}
                    disabled={!canSubmit}
                  >
                    接受简报并开始评审
                  </button>
                )}
                {focus.reviews.length > 0 && (
                  <section>
                    <h3>评审报告</h3>
                    <div className="card-grid">
                      {focus.reviews.map((review) => (
                        <article className="card" key={review.agentCode}>
                          <h4>{review.agentCode}</h4>
                          <strong>发现</strong>
                          <ul>
                            {review.findings.map((item) => (
                              <li key={item}>{item}</li>
                            ))}
                          </ul>
                          <strong>风险</strong>
                          <ul>
                            {review.risks.map((item) => (
                              <li key={item}>{item}</li>
                            ))}
                          </ul>
                          <strong>建议</strong>
                          <ul>
                            {review.recommendations.map((item) => (
                              <li key={item}>{item}</li>
                            ))}
                          </ul>
                        </article>
                      ))}
                    </div>
                  </section>
                )}
                {focus.conflicts.length > 0 && (
                  <section>
                    <h3>冲突决策</h3>
                    {focus.conflicts.map((conflict) => {
                      const decision = focus.decisions.find(
                        (item) => item.conflictKey === conflict.key,
                      );
                      return (
                        <article className="card conflict" key={conflict.key}>
                          <h4>{conflict.description}</h4>
                          {conflict.highImpact && (
                            <span className="warning">高影响</span>
                          )}
                          <div className="choices">
                            {conflict.options.map((option) => (
                              <button
                                key={option}
                                className={
                                  decision?.selectedOption === option
                                    ? "selected"
                                    : undefined
                                }
                                disabled={!canSubmit}
                                onClick={() =>
                                  void decide("recordConflictDecision", {
                                    userDecision: {
                                      conflictKey: conflict.key,
                                      selectedOption: option,
                                      note: null,
                                    },
                                  })
                                }
                              >
                                {option}
                              </button>
                            ))}
                          </div>
                        </article>
                      );
                    })}
                  </section>
                )}
                {focus.artBibleDraft && (
                  <section>
                    <div className="panel-heading">
                      <h3>Art Bible 草案</h3>
                      {focus.workflow.state === "USER_REVIEW" && (
                        <button
                          disabled={
                            !canSubmit ||
                            focus.conflicts.some(
                              (conflict) =>
                                conflict.highImpact &&
                                !focus.decisions.some(
                                  (decision) =>
                                    decision.conflictKey === conflict.key,
                                ),
                            )
                          }
                          onClick={() =>
                            void decide("confirmArtBible", {
                              artBibleMarkdown: focus.artBibleDraft,
                            })
                          }
                        >
                          确认 Art Bible
                        </button>
                      )}
                      {focus.workflow.state === "CONFIRMED" && (
                        <button
                          onClick={() => void decide("versionArtBible")}
                          disabled={!canSubmit}
                        >
                          完成版本化
                        </button>
                      )}
                    </div>
                    <pre className="document">{focus.artBibleDraft}</pre>
                  </section>
                )}
                {artBible && (
                  <section>
                    <h3>历史版本 v{selectedVersion}</h3>
                    <pre className="document">{artBible}</pre>
                  </section>
                )}
                <section>
                  <h3>任务</h3>
                  <div className="task-list">
                    {tasks.map((task) => (
                      <span key={task.id} data-status={task.status}>
                        {task.agentCode} · {task.status}
                      </span>
                    ))}
                  </div>
                </section>
              </section>
            )}
            <details>
              <summary>运行事件（{events.length}）</summary>
              {events.map((event, index) => (
                <pre key={index}>{JSON.stringify(event, null, 2)}</pre>
              ))}
            </details>
          </div>
          <footer>
            <textarea
              value={message}
              onChange={(event) => setMessage(event.target.value)}
              placeholder="输入游戏创意或补充信息"
              disabled={!canSubmit || !conversationId}
            />
            <button
              onClick={() => void submit()}
              disabled={!canSubmit || !conversationId || !message.trim()}
            >
              提交
            </button>
          </footer>
        </section>
      </section>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
