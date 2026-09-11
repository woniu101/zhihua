import type { AspectRatio } from "../domain/storyboard";
import { invokeNative } from "./nativeBridge";

export type FrameFitMode = "cover" | "contain";
export type FrameBackgroundMode = "edge" | "blur" | "solid";

export interface FrameComposition {
  id: string;
  projectId: string;
  assetId: string;
  assetVersionId: string;
  aspectRatio: Exclude<AspectRatio, "auto">;
  fitMode: FrameFitMode;
  focalX: number;
  focalY: number;
  backgroundMode: FrameBackgroundMode;
  workWidth: number;
  workHeight: number;
  visibleWidth: number;
  visibleHeight: number;
  cropX: number;
  cropY: number;
  derivativePath?: string;
  derivativeSha256?: string;
  updatedAt: string;
}

export interface FrameCompositionKey {
  projectId: string;
  assetId: string;
  assetVersionId: string;
  aspectRatio: AspectRatio;
}

export const frameCompositionRepository = {
  get: (input: FrameCompositionKey) =>
    invokeNative<FrameComposition | null>("get_frame_composition", { input }),
  save: (input: FrameCompositionKey & {
    fitMode: FrameFitMode;
    focalX: number;
    focalY: number;
    backgroundMode: FrameBackgroundMode;
  }) => invokeNative<FrameComposition>("save_frame_composition", { input }),
};
