import { convertFileSrc } from "@tauri-apps/api/core";
import type { AssetItem, AssetMediaType, AssetVersion } from "../domain/assets";
import { invokeNative } from "./nativeBridge";

interface NativeAssetVersion {
  id: string;
  versionNumber: number;
  originalFilename: string;
  storedPath: string;
  format: string;
  sizeBytes: number;
  sha256: string;
  note: string;
  createdAt: string;
}

interface NativeAsset {
  id: string;
  projectId: string;
  name: string;
  category: AssetItem["category"];
  mediaType: AssetMediaType;
  source: AssetItem["source"];
  description: string;
  currentVersionId: string;
  versions: NativeAssetVersion[];
  linkedSceneIds: string[];
}

function versionFromNative(value: NativeAssetVersion, mediaType: AssetMediaType): AssetVersion {
  return {
    id: value.id,
    label: `V${value.versionNumber}`,
    fileName: value.originalFilename,
    format: value.format,
    createdAt: value.createdAt,
    note: value.note,
    storedPath: value.storedPath,
    sizeBytes: value.sizeBytes,
    sha256: value.sha256,
    previewUrl: mediaType === "image" ? convertFileSrc(value.storedPath) : undefined,
  };
}

function fromNative(value: NativeAsset): AssetItem {
  return {
    id: value.id,
    name: value.name,
    category: value.category,
    mediaType: value.mediaType,
    source: value.source,
    description: value.description,
    fallbackImage: value.mediaType === "audio" ? "audio" : "clouds",
    currentVersionId: value.currentVersionId,
    versions: value.versions.map((version) => versionFromNative(version, value.mediaType)),
    linkedShotIds: value.linkedSceneIds,
  };
}

export const assetRepository = {
  async list(projectId: string): Promise<AssetItem[]> {
    const result = await invokeNative<NativeAsset[]>("list_assets", { projectId });
    return result?.map(fromNative) ?? [];
  },
  async importPaths(projectId: string, paths: string[]): Promise<AssetItem[]> {
    const result = await invokeNative<NativeAsset[]>("import_asset_files", {
      input: { projectId, paths },
    });
    return result?.map(fromNative) ?? [];
  },
};
