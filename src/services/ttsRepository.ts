import { convertFileSrc } from "@tauri-apps/api/core";
import { invokeNative } from "./nativeBridge";

export interface SystemVoice {
  id: string;
  name: string;
  locale: string;
}

export interface NarrationArtifact {
  projectId: string;
  sceneId: string;
  voiceId: string;
  localPath: string;
  previewUrl: string;
  durationMs: number;
  sizeBytes: number;
  sha256: string;
  textSha256: string;
  updatedAt: string;
}

interface NativeNarrationArtifact extends Omit<NarrationArtifact, "previewUrl"> {}

function fromNative(value: NativeNarrationArtifact): NarrationArtifact {
  return { ...value, previewUrl: convertFileSrc(value.localPath) };
}

export const ttsRepository = {
  async listVoices(): Promise<SystemVoice[]> {
    return (await invokeNative<SystemVoice[]>("list_system_voices")) ?? [];
  },
  async get(projectId: string, sceneId: string): Promise<NarrationArtifact | undefined> {
    const value = await invokeNative<NativeNarrationArtifact | null>("get_scene_narration", { projectId, sceneId });
    return value ? fromNative(value) : undefined;
  },
  async synthesize(input: {
    projectId: string;
    sceneId: string;
    voiceId: string;
    rate: number;
    volume: number;
  }): Promise<NarrationArtifact> {
    const value = await invokeNative<NativeNarrationArtifact>("synthesize_scene_narration", { input });
    if (!value) throw new Error("系统旁白只能在桌面客户端中生成");
    return fromNative(value);
  },
  async importAudio(projectId: string, sceneId: string, assetId: string): Promise<NarrationArtifact> {
    const value = await invokeNative<NativeNarrationArtifact>("import_scene_narration", {
      input: { projectId, sceneId, assetId },
    });
    if (!value) throw new Error("旁白录音只能在桌面客户端中导入");
    return fromNative(value);
  },
};
