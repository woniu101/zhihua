export type AspectRatio = "auto" | "1:1" | "4:3" | "3:4" | "16:9" | "9:16";
export type GenerationMode = "t2v" | "i2v" | "flf2v" | "r2v" | "continue";
export type CandidateQuality = "fast" | "high";
export type H3AudioPolicy = "smart" | "always" | "off";
export type NarrationMode = "tts" | "imported" | "none";
export type AudioIntent = "environment" | "dialogue" | "silent";
export type PromptMode = "quick" | "advanced";
export type SceneDurationSeconds = 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15;
export type SceneStatus = "draft" | "ready" | "generating" | "generated" | "approved" | "failed";

export interface SourceReference {
  sourceId: string;
  page?: number;
  paragraph?: number;
  quote?: string;
}

export interface VisualIntent {
  subject: string;
  action: string;
  scene: string;
  composition: string;
  camera: string;
  lighting: string;
  timeline: string;
  negative: string;
}

export interface SceneDraft {
  id: string;
  projectId: string;
  order: number;
  title: string;
  purpose: string;
  sourceRefs: SourceReference[];
  narration: string;
  narrationMode: NarrationMode;
  ambientSound: string;
  onScreenText: string[];
  visualPlan: string;
  visualIntent: VisualIntent;
  promptMode: PromptMode;
  audioIntent: AudioIntent;
  locked: boolean;
  generationMode: GenerationMode;
  targetDurationMs: number;
  assetIds: string[];
  selectedVersionId?: string;
  lastJobId?: string;
  lastUpscaleJobId?: string;
  pendingRequestId?: string;
  generationStage?: string;
  status: SceneStatus;
  quality: CandidateQuality;
  updatedAt: string;
}

export interface StoryboardProjectSettings {
  aspectRatio: AspectRatio;
  h3AudioPolicy: H3AudioPolicy;
}
