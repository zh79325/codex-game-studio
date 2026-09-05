import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { conversationApi } from "../api";

export type ConversationTarget = {
  projectId: string;
  targetKind: "project" | "character";
  targetRef: string | null;
  title: string;
};

export function useConversation(target: ConversationTarget, enabled = true) {
  const queryClient = useQueryClient();
  const [streamingText, setStreamingText] = useState("");
  const [thinkingText, setThinkingText] = useState("");
  const [workingAgentCode, setWorkingAgentCode] = useState<string>();
  const activeTurnIdRef = useRef<string | undefined>(undefined);
  const pendingThinkingByTurnRef = useRef(new Map<string, string>());
  const [lastError, setLastError] = useState<string>();
  const ensure = useQuery({
    queryKey: [
      "conversation-ensure",
      target.projectId,
      target.targetKind,
      target.targetRef,
    ],
    queryFn: () =>
      conversationApi.ensure(
        target.projectId,
        target.targetKind,
        target.targetRef,
        target.title,
      ),
    enabled: enabled && Boolean(target.projectId),
  });
  const conversationId = ensure.data?.id;
  const snapshot = useQuery({
    queryKey: ["conversation", conversationId],
    queryFn: () => conversationApi.read(conversationId!),
    enabled: Boolean(conversationId),
    refetchInterval: (query) =>
      query.state.data?.conversation.status === "running" ? 1_000 : false,
  });

  const refresh = useCallback(async () => {
    if (!conversationId) return;
    await queryClient.invalidateQueries({
      queryKey: ["conversation", conversationId],
    });
  }, [conversationId, queryClient]);

  useEffect(() => {
    if (!conversationId) return;
    activeTurnIdRef.current = undefined;
    pendingThinkingByTurnRef.current.clear();
    setStreamingText("");
    setThinkingText("");
    return window.codexGame.onEvent((event) => {
      if (typeof event !== "object" || !event || !("method" in event)) return;
      const notification = event as {
        method: string;
        params?: Record<string, unknown>;
      };
      const method = String(notification.method);
      const params = notification.params;
      if (method === "item/reasoning/summaryTextDelta") {
        const turnId = typeof params?.turnId === "string" ? params.turnId : "";
        const delta = typeof params?.delta === "string" ? params.delta : "";
        if (!turnId || !delta) return;
        if (turnId === activeTurnIdRef.current) {
          setThinkingText((current) => current + delta);
        } else {
          const pending = pendingThinkingByTurnRef.current;
          pending.set(turnId, (pending.get(turnId) ?? "") + delta);
        }
        return;
      }
      if (params?.conversationId !== conversationId) return;
      if (method === "game/conversation/delta") {
        const delta = typeof params.delta === "string" ? params.delta : "";
        setStreamingText((current) => current + delta);
        return;
      }
      if (method === "game/conversation/actor") {
        const isWorking = params.status === "working";
        const turnId =
          typeof params.turnId === "string" ? params.turnId : undefined;
        const startsNewTurn =
          isWorking && turnId && turnId !== activeTurnIdRef.current;
        activeTurnIdRef.current = isWorking ? turnId : undefined;
        if (startsNewTurn) {
          setStreamingText("");
          setThinkingText("");
        }
        setWorkingAgentCode(
          isWorking && typeof params.agentCode === "string"
            ? params.agentCode
            : undefined,
        );
        if (isWorking && turnId) {
          const pending = pendingThinkingByTurnRef.current;
          setThinkingText(pending.get(turnId) ?? "");
          pending.delete(turnId);
        }
      }
      if (
        method === "game/conversation/error" &&
        typeof params.message === "string"
      ) {
        setLastError(params.message);
      }
      if (method === "game/conversation/turn" && params.status !== "running") {
        activeTurnIdRef.current = undefined;
        setStreamingText("");
        setThinkingText("");
        setWorkingAgentCode(undefined);
      }
      if (
        method.startsWith("game/conversation/") ||
        method.startsWith("game/task/") ||
        method.startsWith("game/attempt/")
      ) {
        void refresh();
      }
    });
  }, [conversationId, refresh]);

  const sendMutation = useMutation({
    mutationFn: ({
      content,
      recipientAgentCode,
    }: {
      content: string;
      recipientAgentCode?: string;
    }) => conversationApi.send(conversationId!, content, recipientAgentCode),
    onMutate: () => {
      activeTurnIdRef.current = undefined;
      pendingThinkingByTurnRef.current.clear();
      setStreamingText("");
      setThinkingText("");
      setLastError(undefined);
    },
    onSuccess: async (next) => {
      queryClient.setQueryData(["conversation", conversationId], next);
      await refresh();
    },
  });
  const interruptMutation = useMutation({
    mutationFn: () => conversationApi.interrupt(conversationId!),
    onSuccess: refresh,
  });
  const commitDraftsMutation = useMutation({
    mutationFn: (draftIds: string[]) =>
      conversationApi.commitDrafts(conversationId!, draftIds),
    onSuccess: refresh,
  });

  return {
    conversationId,
    snapshot: snapshot.data,
    isLoading: ensure.isLoading || snapshot.isLoading,
    error: ensure.error ?? snapshot.error,
    isBusy:
      snapshot.data?.conversation.status === "running" ||
      sendMutation.isPending,
    isInterrupting: interruptMutation.isPending,
    streamingText,
    thinkingText,
    workingAgentCode,
    lastError,
    send: (content: string, recipientAgentCode?: string) =>
      sendMutation.mutateAsync({ content, recipientAgentCode }),
    interrupt: () => interruptMutation.mutateAsync(),
    commitDrafts: (draftIds: string[]) =>
      commitDraftsMutation.mutateAsync(draftIds),
    refresh,
  };
}
