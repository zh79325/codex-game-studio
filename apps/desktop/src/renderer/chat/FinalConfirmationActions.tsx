import { App, Button, Input, Space, Typography } from "antd";
import { useState } from "react";

export default function FinalConfirmationActions({
  confirming = false,
  onConfirm,
  onSupplement,
}: {
  confirming?: boolean;
  onConfirm: () => Promise<unknown>;
  onSupplement: (content: string) => Promise<unknown>;
}) {
  const { message } = App.useApp();
  const [supplement, setSupplement] = useState("");
  const [submittingConfirmation, setSubmittingConfirmation] = useState(false);
  const [submittingSupplement, setSubmittingSupplement] = useState(false);
  const confirmationBusy = confirming || submittingConfirmation;

  const submitConfirmation = async () => {
    if (confirmationBusy) return;
    setSubmittingConfirmation(true);
    try {
      await onConfirm();
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setSubmittingConfirmation(false);
    }
  };

  const submitSupplement = async () => {
    const content = supplement.trim();
    if (!content) {
      message.warning("请先填写需要补充的内容");
      return;
    }
    if (submittingSupplement) return;
    setSubmittingSupplement(true);
    try {
      await onSupplement(content);
      setSupplement("");
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setSubmittingSupplement(false);
    }
  };

  return (
    <section className="final-confirmation-actions">
      <Typography.Title level={4}>补充要求</Typography.Title>
      <Typography.Text type="secondary">
        如果当前方案还需要调整，请填写具体补充内容后提交。
      </Typography.Text>
      <Input.TextArea
        value={supplement}
        autoSize={{ minRows: 3, maxRows: 6 }}
        placeholder="输入需要补充或调整的内容"
        onChange={(event) => setSupplement(event.target.value)}
      />
      <Space className="final-confirmation-buttons" wrap>
        <Button
          type="primary"
          loading={confirmationBusy}
          onClick={() => void submitConfirmation()}
        >
          没问题，确认设定
        </Button>
        <Button
          loading={submittingSupplement}
          onClick={() => void submitSupplement()}
        >
          我还有需要补充的
        </Button>
      </Space>
    </section>
  );
}
