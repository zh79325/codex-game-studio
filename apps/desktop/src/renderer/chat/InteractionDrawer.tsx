import { App, Card, Drawer, Typography } from "antd";
import { useMemo, useState } from "react";
import type {
  ArtifactDraft,
  ChoiceGroup,
  FeedbackImage,
  Generation,
} from "../types";
import ChoiceQuestions from "./ChoiceQuestions";
import type { ChoiceSubmission } from "./ChoiceQuestions";
import FinalConfirmationActions from "./FinalConfirmationActions";
import MarkdownDocument from "./MarkdownDocument";
import MediaConfirmationPanel from "./MediaConfirmationPanel";

type PendingChoice = {
  id: string;
  groups: ChoiceGroup[];
};

export default function InteractionDrawer({
  choice,
  drafts,
  mediaGenerations = [],
  disabled,
  onSubmitChoice,
  onSubmitFeedback,
  onCommitDrafts,
  onConfirmDraft,
  confirmingDraft = false,
  onConfirmMedia,
  onSubmitMediaFeedback,
  confirmingMedia = false,
}: {
  choice?: PendingChoice;
  drafts: ArtifactDraft[];
  mediaGenerations?: Generation[];
  disabled: boolean;
  onSubmitChoice: (
    choice: PendingChoice,
    submission: ChoiceSubmission,
  ) => Promise<boolean>;
  onSubmitFeedback: (content: string) => Promise<boolean>;
  onCommitDrafts?: (draftIds: string[]) => Promise<unknown>;
  onConfirmDraft?: (draft: ArtifactDraft) => Promise<unknown>;
  confirmingDraft?: boolean;
  onConfirmMedia?: (generation: Generation) => Promise<unknown>;
  onSubmitMediaFeedback?: (
    generation: Generation,
    content: string,
    image?: FeedbackImage,
  ) => Promise<unknown>;
  confirmingMedia?: boolean;
}) {
  const { message } = App.useApp();
  const visibleDrafts = useMemo(
    () => (choice ? [] : drafts.filter((draft) => !isJsonDraft(draft))),
    [choice, drafts],
  );
  const dedicatedDraft = visibleDrafts.find(isDedicatedGateDraft);
  const visibleMediaGenerations =
    choice || visibleDrafts.length ? [] : mediaGenerations;
  const interactionKey = [
    choice?.id ?? "",
    ...visibleDrafts.map((draft) => draft.id),
    ...visibleMediaGenerations.map((generation) => generation.id),
  ].join(":");
  const [dismissedKey, setDismissedKey] = useState("");
  const open =
    Boolean(choice || visibleDrafts.length || visibleMediaGenerations.length) &&
    dismissedKey !== interactionKey;

  const closeDrawer = () => setDismissedKey(interactionKey);
  const submitChoice = async (submission: ChoiceSubmission) => {
    if (!choice) return;
    try {
      if (await onSubmitChoice(choice, submission)) closeDrawer();
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  };
  const submitFeedback = async (content: string) => {
    if (await onSubmitFeedback(content)) closeDrawer();
  };
  const confirmDrafts = async () => {
    if (dedicatedDraft) {
      if (!onConfirmDraft) throw new Error("当前内容缺少确认操作");
      await onConfirmDraft(dedicatedDraft);
    } else if (onCommitDrafts) {
      await onCommitDrafts(visibleDrafts.map((draft) => draft.id));
    } else {
      throw new Error("当前内容缺少确认操作");
    }
    closeDrawer();
  };
  const confirmMedia = async (generation: Generation) => {
    if (!onConfirmMedia) throw new Error("当前媒体缺少确认操作");
    await onConfirmMedia(generation);
    closeDrawer();
  };
  const submitMediaFeedback = async (
    generation: Generation,
    content: string,
    image?: FeedbackImage,
  ) => {
    if (!onSubmitMediaFeedback) throw new Error("当前媒体缺少补充操作");
    await onSubmitMediaFeedback(generation, content, image);
    closeDrawer();
  };

  return (
    <Drawer
      rootClassName="interaction-drawer"
      title="待完成交互"
      placement="bottom"
      size="min(78dvh, 760px)"
      open={open}
      closable={false}
      maskClosable={false}
      keyboard={false}
      onClose={() => undefined}
    >
      <div className="interaction-drawer-content">
        {choice && (
          <section className="interaction-section">
            <Typography.Title level={4}>请选择</Typography.Title>
            <ChoiceQuestions
              key={choice.id}
              groups={choice.groups}
              disabled={disabled}
              onSubmit={submitChoice}
            />
          </section>
        )}
        {visibleDrafts.length > 0 && (
          <DraftConfirmationPanel drafts={visibleDrafts} />
        )}
        {visibleDrafts.length > 0 && (
          <FinalConfirmationActions
            confirming={confirmingDraft}
            disabled={disabled}
            onConfirm={confirmDrafts}
            onSupplement={submitFeedback}
          />
        )}
        {visibleMediaGenerations.length > 0 && (
          <MediaConfirmationPanel
            generations={visibleMediaGenerations}
            disabled={disabled}
            confirming={confirmingMedia}
            onConfirm={confirmMedia}
            onSupplement={submitMediaFeedback}
          />
        )}
      </div>
    </Drawer>
  );
}

function DraftConfirmationPanel({ drafts }: { drafts: ArtifactDraft[] }) {
  return (
    <section className="interaction-section draft-panel">
      <Typography.Title level={4}>最终确认</Typography.Title>
      {drafts.map((draft) => (
        <Card size="small" key={draft.id} title={draft.targetPath}>
          {isMarkdownDraft(draft) ? (
            <MarkdownDocument content={draft.content} />
          ) : (
            <pre className="document draft-document">{draft.content}</pre>
          )}
        </Card>
      ))}
    </section>
  );
}

function isJsonDraft(draft: ArtifactDraft) {
  return draft.targetPath.toLowerCase().endsWith(".json");
}

function isMarkdownDraft(draft: ArtifactDraft) {
  const path = draft.targetPath.toLowerCase();
  return path.endsWith(".md") || path.endsWith(".markdown");
}

function isDedicatedGateDraft(draft: ArtifactDraft) {
  return (
    draft.targetPath === "art-bible.md" ||
    draft.targetPath === "docs/角色定稿.md"
  );
}
