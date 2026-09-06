import { useQuery } from "@tanstack/react-query";
import { Alert, Image, Radio, Space, Spin, Tag, Typography } from "antd";
import { useEffect, useMemo, useState } from "react";
import { charactersApi } from "../api";
import type { FeedbackImage, Generation } from "../types";
import FinalConfirmationActions from "./FinalConfirmationActions";

export default function MediaConfirmationPanel({
  generations,
  disabled,
  confirming,
  onConfirm,
  onSupplement,
}: {
  generations: Generation[];
  disabled: boolean;
  confirming: boolean;
  onConfirm: (generation: Generation) => Promise<unknown>;
  onSupplement: (
    generation: Generation,
    content: string,
    image?: FeedbackImage,
  ) => Promise<unknown>;
}) {
  const ordered = useMemo(
    () => [...generations].sort((left, right) => right.createdAt - left.createdAt),
    [generations],
  );
  const [selectedId, setSelectedId] = useState<string>();
  const selected =
    ordered.find((generation) => generation.id === selectedId) ?? ordered[0];

  useEffect(() => {
    if (!selected || selected.id === selectedId) return;
    setSelectedId(selected.id);
  }, [selected, selectedId]);

  const media = useQuery({
    queryKey: ["generation-media", selected?.projectId, selected?.id],
    queryFn: () =>
      charactersApi.readGenerationMedia(selected!.projectId, selected!.id),
    enabled: Boolean(selected),
    staleTime: Number.POSITIVE_INFINITY,
  });
  const dataUrl = media.data
    ? `data:${media.data.mimeType};base64,${media.data.dataBase64}`
    : undefined;

  if (!selected) return null;

  return (
    <section className="interaction-section media-confirmation-panel">
      <div className="media-confirmation-heading">
        <Typography.Title level={4}>
          {selected.stage === "render" ? "确认角色效果图" : "确认角色四视图"}
        </Typography.Title>
        <Space wrap>
          <Tag color="processing">
            {selected.stage === "render" ? "效果图" : "四视图"}
          </Tag>
          <Typography.Text type="secondary">
            生成于 {formatGenerationTime(selected.createdAt)}
          </Typography.Text>
        </Space>
      </div>

      {ordered.length > 1 && (
        <Radio.Group
          className="media-candidate-switcher"
          value={selected.id}
          disabled={disabled}
          onChange={(event) => setSelectedId(String(event.target.value))}
        >
          {ordered.map((generation, index) => (
            <Radio.Button key={generation.id} value={generation.id}>
              候选 {index + 1}
            </Radio.Button>
          ))}
        </Radio.Group>
      )}

      <div className="media-preview-stage">
        {media.isLoading && <Spin tip="正在加载生成结果" />}
        {media.error && (
          <Alert
            type="error"
            showIcon
            title="生成结果加载失败"
            description={
              media.error instanceof Error ? media.error.message : String(media.error)
            }
          />
        )}
        {dataUrl && media.data?.mimeType.startsWith("image/") && (
          <Image src={dataUrl} alt="待确认的角色生成结果" />
        )}
        {dataUrl && media.data?.mimeType.startsWith("video/") && (
          <video src={dataUrl} controls aria-label="待确认的角色生成结果" />
        )}
      </div>

      <FinalConfirmationActions
        confirming={confirming}
        disabled={disabled || media.isLoading || Boolean(media.error)}
        allowImage
        confirmLabel={
          selected.stage === "render" ? "没问题，确认效果" : "没问题，确认四视图"
        }
        onConfirm={() => onConfirm(selected)}
        onSupplement={(content, image) => onSupplement(selected, content, image)}
      />
    </section>
  );
}

function formatGenerationTime(timestamp: number) {
  const milliseconds = timestamp < 1_000_000_000_000 ? timestamp * 1_000 : timestamp;
  return new Date(milliseconds).toLocaleString();
}
