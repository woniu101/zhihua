import { open } from "@tauri-apps/plugin-dialog";
import { invokeNative } from "./nativeBridge";

export type ExportStatus = "idle" | "checking" | "blocked" | "exporting" | "succeeded" | "failed";

export interface ExportSettings {
  container: "MP4";
  videoCodec: "H.264";
  audioCodec: "AAC";
  frameRate: 24 | 25 | 30;
  ratio: "16:9";
  resolution: "1920 × 1080";
  subtitleMode: "burn-and-srt" | "burn" | "srt";
  musicAssetId: string;
  outputDirectory: string;
}

export interface IntegrityInput {
  totalShots: number;
  readyShotIds: string[];
  narrationComplete: boolean;
  narrationIssueCount: number;
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
  ffmpegPath?: string;
  version?: string;
}

export interface ProjectExport {
  outputPath: string;
  subtitlePath?: string;
  durationMs: number;
  sizeBytes: number;
  sha256: string;
  createdAt: string;
}

export const defaultExportSettings = (): ExportSettings => ({
  container: "MP4",
  videoCodec: "H.264",
  audioCodec: "AAC",
  frameRate: 24,
  ratio: "16:9",
  resolution: "1920 × 1080",
  subtitleMode: "burn-and-srt",
  musicAssetId: "",
  outputDirectory: "",
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
      detail: input.narrationComplete && input.subtitleComplete
        ? "已检查"
        : input.narrationIssueCount
          ? `${input.narrationIssueCount} 个旁白需重生成或调整时长`
          : "存在缺失",
      passed: input.narrationComplete && input.subtitleComplete && input.narrationIssueCount === 0,
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
  return (await invokeNative<ExportCapability>("inspect_export_capability")) ?? {
    available: false,
    label: "仅桌面端可导出",
    reason: "请运行知画桌面客户端。",
  };
}

export function estimateOutputSizeMb(durationSeconds: number, frameRate: number): number {
  const videoMbps = frameRate >= 30 ? 18 : 14;
  const audioMbps = 0.192;
  return Math.max(1, Math.round(durationSeconds * (videoMbps + audioMbps) / 8));
}

export async function chooseExportDirectory(): Promise<string | undefined> {
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === "string" ? selected : undefined;
}

export async function requestNativeExport(
  projectId: string,
  settings: ExportSettings,
  narrationVolume: number,
  musicVolume: number,
  musicFade: boolean,
): Promise<ProjectExport> {
  const result = await invokeNative<ProjectExport>("export_project_video", {
    input: {
      projectId,
      outputDirectory: settings.outputDirectory,
      frameRate: settings.frameRate,
      subtitleMode: settings.subtitleMode,
      narrationVolume,
      musicAssetId: settings.musicAssetId || undefined,
      musicVolume,
      musicFade,
    },
  });
  if (!result) throw new Error("视频只能在知画桌面客户端中导出");
  return result;
}
