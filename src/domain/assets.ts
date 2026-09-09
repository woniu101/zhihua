export type AssetCategory = "角色" | "场景" | "道具" | "风格" | "音频";

export type AssetMediaType = "image" | "audio";

export interface AssetVersion {
  id: string;
  label: string;
  fileName: string;
  format: string;
  dimensions?: string;
  duration?: string;
  createdAt: string;
  note: string;
  previewUrl?: string;
}

export interface AssetItem {
  id: string;
  name: string;
  category: AssetCategory;
  mediaType: AssetMediaType;
  source: "本地上传" | "剪贴板" | "演示素材";
  description: string;
  fallbackImage: string;
  currentVersionId: string;
  versions: AssetVersion[];
  linkedShotIds: string[];
}

export const ASSET_CATEGORIES: Array<"全部" | AssetCategory> = [
  "全部",
  "角色",
  "场景",
  "道具",
  "风格",
  "音频",
];

export function currentAssetVersion(asset: AssetItem): AssetVersion {
  return asset.versions.find((version) => version.id === asset.currentVersionId) ?? asset.versions[0];
}

export function stripFileExtension(fileName: string): string {
  const value = fileName.replace(/\.[^/.]+$/, "").trim();
  return value || "未命名素材";
}
