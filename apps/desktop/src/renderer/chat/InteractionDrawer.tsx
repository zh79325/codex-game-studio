import { Button, Card, Checkbox, Drawer, Typography } from "antd";
import { App } from "antd";
import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { ArtifactDraft, ChoiceGroup } from "../types";
import ChoiceQuestions from "./ChoiceQuestions";
import MarkdownDocument from "./MarkdownDocument";

type PendingChoice = {
  id: string;
  groups: ChoiceGroup[];
};

export default function InteractionDrawer({
  choice,
  drafts,
  disabled,
  onSubmitChoice,
  onCommitDrafts,
  renderDraftAction,
}: {
  choice?: PendingChoice;
  drafts: ArtifactDraft[];
  disabled: boolean;
  onSubmitChoice: (content: string) => Promise<boolean>;
  onCommitDrafts?: (draftIds: string[]) => Promise<unknown>;
  renderDraftAction?: (
    draft: ArtifactDraft,
    closeDrawer: () => void,
  ) => ReactNode;
}) {
  const { message } = App.useApp();
  const visibleDrafts = useMemo(
    () => drafts.filter((draft) => !isJsonDraft(draft)),
    [drafts],
  );
  const committableDrafts = visibleDrafts.filter(
    (draft) => !isDedicatedGateDraft(draft),
  );
  const interactionKey = [
    choice?.id ?? "",
    ...visibleDrafts.map((draft) => draft.id),
  ].join(":");
  const [dismissedKey, setDismissedKey] = useState("");
  const [selectedDrafts, setSelectedDrafts] = useState<string[]>([]);
  const open =
    Boolean(choice || visibleDrafts.length) && dismissedKey !== interactionKey;

  useEffect(() => {
    const availableIds = new Set(committableDrafts.map((draft) => draft.id));
    setSelectedDrafts((current) =>
      current.filter((id) => availableIds.has(id)),
    );
  }, [interactionKey]);

  const closeDrawer = () => setDismissedKey(interactionKey);
  const submitChoice = async (content: string) => {
    if (await onSubmitChoice(content)) closeDrawer();
  };
  const commitDrafts = async () => {
    if (!onCommitDrafts || !selectedDrafts.length) return;
    try {
      await onCommitDrafts(selectedDrafts);
      setSelectedDrafts([]);
      closeDrawer();
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <Drawer
      rootClassName="interaction-drawer"
      title="待完成交互"
      placement="bottom"
      height="min(78dvh, 760px)"
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
          <DraftConfirmationPanel
            drafts={visibleDrafts}
            selected={selectedDrafts}
            disabled={disabled}
            onSelected={setSelectedDrafts}
            onCommit={
              onCommitDrafts && committableDrafts.length
                ? commitDrafts
                : undefined
            }
            renderAction={renderDraftAction}
            closeDrawer={closeDrawer}
          />
        )}
      </div>
    </Drawer>
  );
}

function DraftConfirmationPanel({
  drafts,
  selected,
  disabled,
  onSelected,
  onCommit,
  renderAction,
  closeDrawer,
}: {
  drafts: ArtifactDraft[];
  selected: string[];
  disabled: boolean;
  onSelected: (ids: string[]) => void;
  onCommit?: () => Promise<void>;
  renderAction?: (draft: ArtifactDraft, closeDrawer: () => void) => ReactNode;
  closeDrawer: () => void;
}) {
  return (
    <section className="interaction-section draft-panel">
      <Typography.Title level={4}>素材确认</Typography.Title>
      {drafts.map((draft) => (
        <Card
          size="small"
          key={draft.id}
          title={draft.targetPath}
          extra={renderAction?.(draft, closeDrawer)}
        >
          {!isDedicatedGateDraft(draft) && (
            <Checkbox
              checked={selected.includes(draft.id)}
              disabled={disabled || !onCommit}
              onChange={(event) =>
                onSelected(
                  event.target.checked
                    ? [...selected, draft.id]
                    : selected.filter((id) => id !== draft.id),
                )
              }
            >
              选择提交
            </Checkbox>
          )}
          {isMarkdownDraft(draft) ? (
            <MarkdownDocument content={draft.content} />
          ) : (
            <pre className="document draft-document">{draft.content}</pre>
          )}
        </Card>
      ))}
      {onCommit && (
        <Button
          type="primary"
          disabled={disabled || !selected.length}
          onClick={() => void onCommit()}
        >
          提交所选草稿
        </Button>
      )}
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
