export type ExportStatus = "idle" | "checking" | "blocked" | "exporting" | "succeeded" | "failed";

export interface ExportSettings {
  container: "MP4";
  videoCodec: "H.264";
  audioCodec: "AAC";
  frameRate: 24 | 25 | 30;
  ratio: "16:9";
  resolution: "1920 × 1080";
  subtitleMode: "burn-and-srt" | "burn" | "srt";
  outputDirectory: string;
}

export interface IntegrityInput {
  totalShots: number;
  readyShotIds: string[];
  narrationComplete: boolean;
  subtitleComplete: boolean;
  sourceRecordsComplete: boolean;
  missingAssetNames: string[];
}

export interface IntegrityCheckItem {
  id: string;
  label: string;
  detail: string;
  passed: boolean;
}

export interface ExportCapability {
  available: boolean;
  label: string;
  reason: string;
}

export const defaultExportSettings = (): ExportSettings => ({
  container: "MP4",
  videoCodec: "H.264",
  audioCodec: "AAC",
  frameRate: 24,
  ratio: "16:9",
  resolution: "1920 × 1080",
  subtitleMode: "burn-and-srt",
  outputDirectory: "D:\\知画\\导出\\闪电科普视频",
});

export function inspectIntegrity(input: IntegrityInput): IntegrityCheckItem[] {
  const readyCount = new Set(input.readyShotIds).size;
  return [
    {
      id: "shots",
      label: "正式 1080p 分镜",
      detail: `${readyCount}/${input.totalShots} 就绪`,
      passed: input.totalShots > 0 && readyCount === input.totalShots,
    },
    {
      id: "narration",
      label: "旁白和字幕完整",
      detail: input.narrationComplete && input.subtitleComplete ? "已检查" : "存在缺失",
      passed: input.narrationComplete && input.subtitleComplete,
    },
    {
      id: "sources",
      label: "来源记录完整",
      detail: input.sourceRecordsComplete ? "已检查" : "需补充来源",
      passed: input.sourceRecordsComplete,
    },
    {
      id: "assets",
      label: "无缺失素材",
      detail: input.missingAssetNames.length ? `缺少 ${input.missingAssetNames.length} 项` : "已检查",
      passed: input.missingAssetNames.length === 0,
    },
  ];
}

export async function inspectExportCapability(): Promise<ExportCapability> {
  return {
    available: false,
    label: "FFmpeg 未连接",
    reason: "桌面端 FFmpeg native command 尚未接入；当前可调整并保存导出参数，但不能执行视频合成。",
  };
}

export function estimateOutputSizeMb(durationSeconds: number, frameRate: number): number {
  const videoMbps = frameRate >= 30 ? 18 : 14;
  const audioMbps = 0.192;
  return Math.max(1, Math.round(durationSeconds * (videoMbps + audioMbps) / 8));
}

export async function requestNativeExport(_settings: ExportSettings): Promise<never> {
  throw new Error("FFmpeg native command 尚未接入，未执行导出。请先在设置与算力页完成 FFmpeg 检测和桌面命令接入。");
}
