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
  async importPayload(projectId: string, filename: string, base64Data: string, source: "本地上传" | "剪贴板"): Promise<AssetItem> {
    const result = await invokeNative<NativeAsset>("import_asset_payload", {
      input: { projectId, filename, base64Data, source },
    });
    if (!result) throw new Error("剪贴板素材只能在桌面客户端中导入。");
    return fromNative(result);
  },
  async update(input: {
    id: string;
    name: string;
    category: AssetItem["category"];
    description: string;
  }): Promise<AssetItem> {
    const result = await invokeNative<NativeAsset>("update_asset", { input });
    if (!result) throw new Error("素材信息只能在桌面客户端中保存。");
    return fromNative(result);
  },
  async replace(assetId: string, sourcePath: string): Promise<AssetItem> {
    const result = await invokeNative<NativeAsset>("replace_asset_file", {
      input: { assetId, sourcePath },
    });
    if (!result) throw new Error("素材文件只能在桌面客户端中替换。");
    return fromNative(result);
  },
  async setCurrentVersion(assetId: string, versionId: string): Promise<AssetItem> {
    const result = await invokeNative<NativeAsset>("set_current_asset_version", {
      input: { assetId, versionId },
    });
    if (!result) throw new Error("素材版本只能在桌面客户端中切换。");
    return fromNative(result);
  },
  delete: (assetId: string) => invokeNative<void>("delete_asset", { assetId }),
  async unlink(assetId: string, sceneId?: string): Promise<AssetItem> {
    const result = await invokeNative<NativeAsset>("unlink_asset", {
      input: { assetId, sceneId },
    });
    if (!result) throw new Error("分镜关联只能在桌面客户端中修改。");
    return fromNative(result);
  },
  async downloadCompletedImage(projectId: string, jobId: string): Promise<AssetItem[]> {
    const result = await invokeNative<NativeAsset[]>("download_completed_image_job", {
      input: { projectId, jobId },
    });
    return result?.map(fromNative) ?? [];
  },
};
