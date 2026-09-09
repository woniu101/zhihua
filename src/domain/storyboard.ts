export type AspectRatio = "auto" | "1:1" | "4:3" | "3:4" | "16:9" | "9:16";
export type GenerationMode = "t2v" | "i2v" | "flf2v" | "r2v" | "continue";
export type CandidateQuality = "fast" | "high";
export type SceneStatus = "draft" | "ready" | "generating" | "generated" | "approved" | "failed";

export interface SourceReference {
  sourceId: string;
  page?: number;
  paragraph?: number;
  quote?: string;
}

export interface SceneDraft {
  id: string;
  projectId: string;
  order: number;
  title: string;
  purpose: string;
  sourceRefs: SourceReference[];
  narration: string;
  onScreenText: string[];
  visualPlan: string;
  generationMode: GenerationMode;
  targetDurationMs: 5000 | 10000 | 15000;
  assetIds: string[];
  selectedVersionId?: string;
  status: SceneStatus;
  quality: CandidateQuality;
  updatedAt: string;
}

export interface StoryboardProjectSettings {
  aspectRatio: AspectRatio;
  outputWidth: number;
  outputHeight: number;
  discardH3Audio: boolean;
}
