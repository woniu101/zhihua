import { convertFileSrc } from "@tauri-apps/api/core";
import { invokeNative } from "./nativeBridge";

export interface CandidateVersion {
  id: string;
  projectId: string;
  sceneId: string;
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
  selected: boolean;
  createdAt: string;
}

export interface FinalVersion {
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
interface NativeFinalVersion extends Omit<FinalVersion, "previewUrl"> {}

function fromNative(value: NativeCandidateVersion): CandidateVersion {
  return {
    ...value,
    previewUrl: convertFileSrc(value.localPath),
  };
}

function finalFromNative(value: NativeFinalVersion): FinalVersion {
  return {
    ...value,
    previewUrl: convertFileSrc(value.localPath),
  };
}

export const generationRepository = {
  async list(projectId: string, sceneId: string): Promise<CandidateVersion[]> {
    const result = await invokeNative<NativeCandidateVersion[]>(
      "list_candidate_versions",
      { input: { projectId, sceneId } },
    );
    return result?.map(fromNative) ?? [];
  },

  async downloadCompletedJob(projectId: string, jobId: string): Promise<CandidateVersion[]> {
    const result = await invokeNative<NativeCandidateVersion[]>(
      "download_completed_job",
      { input: { projectId, jobId } },
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

  async listFinals(projectId: string, sceneId: string): Promise<FinalVersion[]> {
    const result = await invokeNative<NativeFinalVersion[]>(
      "list_final_versions",
      { input: { projectId, sceneId } },
    );
    return result?.map(finalFromNative) ?? [];
  },

  async downloadCompletedUpscale(
    projectId: string,
    jobId: string,
    sourceCandidateId: string,
  ): Promise<FinalVersion[]> {
    const result = await invokeNative<NativeFinalVersion[]>(
      "download_completed_upscale",
      { input: { projectId, jobId, sourceCandidateId } },
    );
    return result?.map(finalFromNative) ?? [];
  },
};
