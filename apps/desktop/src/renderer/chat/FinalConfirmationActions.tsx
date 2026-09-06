import { DeleteOutlined } from "@ant-design/icons";
import { App, Button, Image, Input, Space, Typography } from "antd";
import { useEffect, useState } from "react";
import type { ClipboardEvent as ReactClipboardEvent } from "react";
import type { FeedbackImage } from "../types";

const MAX_FEEDBACK_IMAGE_BYTES = 16 * 1024 * 1024;
const MAX_FEEDBACK_IMAGE_PIXELS = 40_000_000;
const FEEDBACK_IMAGE_MIMES = new Set([
  "image/png",
  "image/jpeg",
  "image/webp",
]);

export default function FinalConfirmationActions({
  confirming = false,
  disabled = false,
  allowImage = false,
  confirmLabel = "没问题，确认设定",
  onConfirm,
  onSupplement,
}: {
  confirming?: boolean;
  disabled?: boolean;
  allowImage?: boolean;
  confirmLabel?: string;
  onConfirm: () => Promise<unknown>;
  onSupplement: (content: string, image?: FeedbackImage) => Promise<unknown>;
}) {
  const { message } = App.useApp();
  const [supplement, setSupplement] = useState("");
  const [feedbackImage, setFeedbackImage] = useState<File>();
  const [previewUrl, setPreviewUrl] = useState<string>();
  const [submittingConfirmation, setSubmittingConfirmation] = useState(false);
  const [submittingSupplement, setSubmittingSupplement] = useState(false);
  const busy = confirming || submittingConfirmation || submittingSupplement;
  const confirmationBusy = confirming || submittingConfirmation;

  useEffect(
    () => () => {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
    },
    [previewUrl],
  );

  const handlePaste = async (event: ReactClipboardEvent<HTMLTextAreaElement>) => {
    if (!allowImage) return;
    const image = Array.from(event.clipboardData.files).find((file) =>
      file.type.startsWith("image/"),
    );
    if (!image) return;
    event.preventDefault();
    try {
      await validateFeedbackImage(image);
      setFeedbackImage(image);
      setPreviewUrl(URL.createObjectURL(image));
    } catch (error) {
      message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const clearFeedbackImage = () => {
    setFeedbackImage(undefined);
    setPreviewUrl(undefined);
  };

  const submitConfirmation = async () => {
    if (busy || disabled) return;
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
    if (busy || disabled) return;
    setSubmittingSupplement(true);
    try {
      await onSupplement(
        content,
        feedbackImage ? await fileToFeedbackImage(feedbackImage) : undefined,
      );
      setSupplement("");
      clearFeedbackImage();
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
        {allowImage
          ? "请填写具体调整要求；也可以直接粘贴一张参考图，继续粘贴会替换当前图片。"
          : "如果当前方案还需要调整，请填写具体补充内容后提交。"}
      </Typography.Text>
      <Input.TextArea
        value={supplement}
        autoSize={{ minRows: 3, maxRows: 6 }}
        disabled={disabled || busy}
        placeholder="输入需要补充或调整的内容"
        onChange={(event) => setSupplement(event.target.value)}
        onPaste={(event) => void handlePaste(event)}
      />
      {allowImage && previewUrl && (
        <div className="feedback-image-preview">
          <Image src={previewUrl} alt="反馈参考图预览" />
          <Button
            icon={<DeleteOutlined />}
            disabled={disabled || busy}
            onClick={clearFeedbackImage}
          >
            移除图片
          </Button>
        </div>
      )}
      <Space className="final-confirmation-buttons" wrap>
        <Button
          type="primary"
          loading={confirmationBusy}
          disabled={disabled || submittingSupplement}
          onClick={() => void submitConfirmation()}
        >
          {confirmLabel}
        </Button>
        <Button
          loading={submittingSupplement}
          disabled={disabled || confirmationBusy}
          onClick={() => void submitSupplement()}
        >
          我还有需要补充的
        </Button>
      </Space>
    </section>
  );
}

async function validateFeedbackImage(file: File) {
  if (!FEEDBACK_IMAGE_MIMES.has(file.type)) {
    throw new Error("仅支持 PNG、JPEG 或 WebP 图片");
  }
  if (file.size > MAX_FEEDBACK_IMAGE_BYTES) {
    throw new Error("反馈图片不能超过 16 MiB");
  }
  const dimensions = await readImageDimensions(file);
  if (dimensions.width * dimensions.height > MAX_FEEDBACK_IMAGE_PIXELS) {
    throw new Error("反馈图片像素不能超过 4000 万");
  }
}

function readImageDimensions(file: File) {
  return new Promise<{ width: number; height: number }>((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const image = new window.Image();
    image.onload = () => {
      URL.revokeObjectURL(url);
      resolve({ width: image.naturalWidth, height: image.naturalHeight });
    };
    image.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("无法读取反馈图片"));
    };
    image.src = url;
  });
}

function fileToFeedbackImage(file: File) {
  return new Promise<FeedbackImage>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = typeof reader.result === "string" ? reader.result : "";
      const separator = dataUrl.indexOf(",");
      if (separator < 0) {
        reject(new Error("无法编码反馈图片"));
        return;
      }
      resolve({
        mimeType: file.type as FeedbackImage["mimeType"],
        dataBase64: dataUrl.slice(separator + 1),
      });
    };
    reader.onerror = () => reject(new Error("无法读取反馈图片"));
    reader.readAsDataURL(file);
  });
}
