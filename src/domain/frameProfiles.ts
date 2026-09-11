import type { AspectRatio } from "./storyboard";

export type ConcreteAspectRatio = Exclude<AspectRatio, "auto">;
export type OutputRendition = "candidate" | "enhanced-1080p";

export interface FrameProfile {
  aspectRatio: ConcreteAspectRatio;
  workWidth: number;
  workHeight: number;
  visibleWidth: number;
  visibleHeight: number;
  cropX: number;
  cropY: number;
  fullHdWidth: number;
  fullHdHeight: number;
}

export const frameProfiles: Record<ConcreteAspectRatio, FrameProfile> = {
  "16:9": { aspectRatio: "16:9", workWidth: 1344, workHeight: 768, visibleWidth: 1344, visibleHeight: 756, cropX: 0, cropY: 6, fullHdWidth: 1920, fullHdHeight: 1080 },
  "9:16": { aspectRatio: "9:16", workWidth: 768, workHeight: 1344, visibleWidth: 756, visibleHeight: 1344, cropX: 6, cropY: 0, fullHdWidth: 1080, fullHdHeight: 1920 },
  "4:3": { aspectRatio: "4:3", workWidth: 1024, workHeight: 768, visibleWidth: 1024, visibleHeight: 768, cropX: 0, cropY: 0, fullHdWidth: 1440, fullHdHeight: 1080 },
  "3:4": { aspectRatio: "3:4", workWidth: 768, workHeight: 1024, visibleWidth: 768, visibleHeight: 1024, cropX: 0, cropY: 0, fullHdWidth: 1080, fullHdHeight: 1440 },
  "1:1": { aspectRatio: "1:1", workWidth: 768, workHeight: 768, visibleWidth: 768, visibleHeight: 768, cropX: 0, cropY: 0, fullHdWidth: 1080, fullHdHeight: 1080 },
};

export function resolveAspectRatio(aspectRatio: AspectRatio): ConcreteAspectRatio {
  return aspectRatio === "auto" ? "16:9" : aspectRatio;
}

export function frameProfile(aspectRatio: AspectRatio): FrameProfile {
  return frameProfiles[resolveAspectRatio(aspectRatio)];
}

export function renditionDimensions(aspectRatio: AspectRatio, rendition: OutputRendition) {
  const profile = frameProfile(aspectRatio);
  return rendition === "enhanced-1080p"
    ? { width: profile.fullHdWidth, height: profile.fullHdHeight }
    : { width: profile.visibleWidth, height: profile.visibleHeight };
}
