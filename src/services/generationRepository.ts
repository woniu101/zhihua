import { convertFileSrc } from "@tauri-apps/api/core";
import { invokeNative } from "./nativeBridge";
import type { FrameProfile } from "../domain/frameProfiles";

export interface CandidateVersion {
  id: string;
  projectId: string;
  sceneId: string;
  jobId: string;
  workflowId: string;
  promptId?: string;
  promptCompilerVersion?: string;
  h3AudioPolicy?: string;
  promptText?: string;
  seed?: number;
  audioIntent?: string;
  targetDurationSec?: number;
  artifactId: string;
  filename: string;
  mediaType: string;
  localPath: string;
  previewUrl: string;
  sizeBytes: number;
  sha256: string;
  selected: boolean;
  createdAt: string;
  aspectRatio: string;
  workWidth: number;
  workHeight: number;
  visibleWidth: number;
  visibleHeight: number;
  cropX: number;
  cropY: number;
}

export interface CandidateAudioInspection {
  candidateId: string;
  hasAudio: boolean;
  speechDetected: boolean;
  voicedDurationMs: number;
  voicedRatio: number;
  peakVoiceProbability: number;
  smartEligible: boolean;
  detail: string;
}

export interface EnhancedVersion {
  id: string;
  projectId: string;
  sceneId: string;
  sourceCandidateId: string;
  jobId: string;
  workflowId: string;
  promptId?: string;
  artifactId: string;
  filename: string;
  mediaType: string;
  localPath: string;
  previewUrl: string;
  sizeBytes: number;
  sha256: string;
  createdAt: string;
}

interface NativeCandidateVersion extends Omit<CandidateVersion, "previewUrl"> {}
interface NativeEnhancedVersion extends Omit<EnhancedVersion, "previewUrl"> {}

function fromNative(value: NativeCandidateVersion): CandidateVersion {
  return {
    ...value,
    previewUrl: convertFileSrc(value.localPath),
  };
}

function enhancedFromNative(value: NativeEnhancedVersion): EnhancedVersion {
  return {
    ...value,
    previewUrl: convertFileSrc(value.localPath),
  };
}

export const generationRepository = {
  openLocation: (candidateId: string) => invokeNative<void>("open_candidate_location", { candidateId }),
  inspectAudio: (candidateId: string) =>
    invokeNative<CandidateAudioInspection>("inspect_candidate_audio", {
      input: { candidateId },
    }),
  async list(projectId: string, sceneId: string): Promise<CandidateVersion[]> {
    const result = await invokeNative<NativeCandidateVersion[]>(
      "list_candidate_versions",
      { input: { projectId, sceneId } },
    );
    return result?.map(fromNative) ?? [];
  },

  async downloadCompletedJob(projectId: string, jobId: string, profile: FrameProfile): Promise<CandidateVersion[]> {
    const result = await invokeNative<NativeCandidateVersion[]>(
      "download_completed_job",
      {
        input: {
          projectId,
          jobId,
          aspectRatio: profile.aspectRatio,
          workWidth: profile.workWidth,
          workHeight: profile.workHeight,
          visibleWidth: profile.visibleWidth,
          visibleHeight: profile.visibleHeight,
          cropX: profile.cropX,
          cropY: profile.cropY,
        },
      },
    );
    return result?.map(fromNative) ?? [];
  },

  async select(projectId: string, sceneId: string, versionId: string): Promise<CandidateVersion> {
    const result = await invokeNative<NativeCandidateVersion>(
      "select_candidate_version",
      { input: { projectId, sceneId, versionId } },
    );
    if (!result) throw new Error("候选版本只能在桌面客户端中选择。");
    return fromNative(result);
  },

  async listEnhanced(projectId: string, sceneId: string): Promise<EnhancedVersion[]> {
    const result = await invokeNative<NativeEnhancedVersion[]>(
      "list_enhanced_versions",
      { input: { projectId, sceneId } },
    );
    return result?.map(enhancedFromNative) ?? [];
  },

  async downloadCompletedEnhancement(
    projectId: string,
    jobId: string,
    sourceCandidateId: string,
  ): Promise<EnhancedVersion[]> {
    const result = await invokeNative<NativeEnhancedVersion[]>(
      "download_completed_enhancement",
      { input: { projectId, jobId, sourceCandidateId } },
    );
    return result?.map(enhancedFromNative) ?? [];
  },
};
