import { computed, reactive } from "vue";
import {
  currentAssetVersion,
  stripFileExtension,
  type AssetCategory,
  type AssetItem,
  type AssetMediaType,
  type AssetVersion,
} from "../domain/assets";
import { assetRepository } from "../services/assetRepository";
import { isNativeRuntime } from "../services/nativeBridge";
import { activeProjectId } from "../services/storyboardRepository";

type ImportSource = "本地上传" | "剪贴板";

interface AssetStoreState {
  items: AssetItem[];
  selectedId: string | null;
  activeCategory: "全部" | AssetCategory;
  query: string;
  detailTab: "versions" | "links";
  notice: string;
}

const nowText = () => new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
}).format(new Date()).replace(/\//g, "-");

const uid = (prefix: string) => `${prefix}-${crypto.randomUUID?.() ?? `${Date.now()}-${Math.random()}`}`;

function version(label: string, fileName: string, dimensions: string | undefined, duration: string | undefined, note: string): AssetVersion {
  return {
    id: uid("version"),
    label,
    fileName,
    format: fileName.split(".").pop()?.toUpperCase() ?? "未知",
    dimensions,
    duration,
    createdAt: "2026-09-08 14:28",
    note,
  };
}

function seedAsset(
  id: string,
  name: string,
  category: AssetCategory,
  fallbackImage: string,
  fileName: string,
  links: string[],
  versionCount = 1,
): AssetItem {
  const mediaType: AssetMediaType = category === "音频" ? "audio" : "image";
  const versions = Array.from({ length: versionCount }, (_, index) => {
    const number = versionCount - index;
    return version(
      `V${number}`,
      fileName,
      mediaType === "image" ? "1920 × 1080" : undefined,
      mediaType === "audio" ? "02:42" : undefined,
      number === 1 ? "初始版本" : "优化了构图、亮度和画面细节",
    );
  });
  return {
    id,
    name,
    category,
    mediaType,
    source: "演示素材",
    description: name === "闪电构图"
      ? "乌云密布的夜空中闪电劈下，突出雷电的强烈感，用于科普分镜的关键画面。"
      : `${name}，用于当前项目的参考素材。`,
    fallbackImage,
    currentVersionId: versions[0].id,
    versions,
    linkedShotIds: links,
  };
}

const state = reactive<AssetStoreState>({
  items: [
    seedAsset("clouds", "雷云场景", "场景", "clouds", "thunder-clouds.png", ["01", "02", "03"]),
    seedAsset("bolt", "闪电构图", "场景", "bolt", "lightning.png", ["01", "02", "03", "04", "05"], 2),
    seedAsset("runner", "安全避险人物", "角色", "runner", "runner.png", ["02", "04", "05"]),
    seedAsset("mountain", "山地背景", "场景", "mountain", "mountain.png", ["01", "03"]),
    seedAsset("palette", "科普配色", "风格", "palette", "palette.png", ["01"]),
    seedAsset("audio", "轻柔科普音乐", "音频", "audio", "science-music.mp3", ["01", "03", "05"]),
    seedAsset("safety", "避险道具组合", "道具", "safety", "safety-kit.png", ["05"]),
    seedAsset("village", "夜晚小镇", "场景", "village", "night-town.png", ["02", "04"]),
    seedAsset("street", "雨夜街道", "场景", "street", "rain-street.png", ["04"]),
  ],
  selectedId: "bolt",
  activeCategory: "全部",
  query: "",
  detailTab: "versions",
  notice: "",
});
let loadedProjectId: string | undefined;
let loadingPromise: Promise<void> | undefined;

const selectedAsset = computed(() => state.items.find((item) => item.id === state.selectedId) ?? null);
const filteredAssets = computed(() => {
  const keyword = state.query.trim().toLocaleLowerCase();
  return state.items.filter((asset) => {
    const categoryMatches = state.activeCategory === "全部" || asset.category === state.activeCategory;
    const keywordMatches = !keyword || `${asset.name} ${asset.category} ${asset.description}`.toLocaleLowerCase().includes(keyword);
    return categoryMatches && keywordMatches;
  });
});

function readImageDimensions(url: string): Promise<string | undefined> {
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => resolve(`${image.naturalWidth} × ${image.naturalHeight}`);
    image.onerror = () => resolve(undefined);
    image.src = url;
  });
}

function readAudioDuration(url: string): Promise<string | undefined> {
  return new Promise((resolve) => {
    const audio = document.createElement("audio");
    audio.preload = "metadata";
    audio.onloadedmetadata = () => {
      const seconds = Number.isFinite(audio.duration) ? Math.round(audio.duration) : 0;
      resolve(`${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`);
    };
    audio.onerror = () => resolve(undefined);
    audio.src = url;
  });
}

async function versionFromFile(file: File, number: number, note: string): Promise<AssetVersion & { mediaType: AssetMediaType }> {
  const mediaType: AssetMediaType = file.type.startsWith("audio/")
    ? "audio"
    : file.type.startsWith("video/") ? "video" : "image";
  const previewUrl = URL.createObjectURL(file);
  const [dimensions, duration] = await Promise.all([
    mediaType === "image" ? readImageDimensions(previewUrl) : Promise.resolve(undefined),
    mediaType === "audio" || mediaType === "video" ? readAudioDuration(previewUrl) : Promise.resolve(undefined),
  ]);
  return {
    id: uid("version"),
    label: `V${number}`,
    fileName: file.name,
    format: file.name.split(".").pop()?.toUpperCase() || file.type.split("/").pop()?.toUpperCase() || "未知",
    dimensions,
    duration,
    createdAt: nowText(),
    note,
    previewUrl,
    mediaType,
  };
}

function acceptedFiles(files: File[]): File[] {
  return files.filter((file) => file.type.startsWith("image/") || file.type.startsWith("audio/") || file.type.startsWith("video/"));
}

function fileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error(`无法读取素材“${file.name}”`));
    reader.onload = () => {
      const value = typeof reader.result === "string" ? reader.result : "";
      const separator = value.indexOf(",");
      if (separator < 0) reject(new Error(`无法编码素材“${file.name}”`));
      else resolve(value.slice(separator + 1));
    };
    reader.readAsDataURL(file);
  });
}

async function loadActiveProject(force = false): Promise<void> {
  if (!isNativeRuntime()) return;
  const projectId = activeProjectId();
  if (!projectId) {
    state.items = [];
    state.selectedId = null;
    state.notice = "请先在项目页打开一个项目";
    return;
  }
  if (!force && loadedProjectId === projectId) return;
  if (loadingPromise) return loadingPromise;
  loadingPromise = (async () => {
    try {
      state.items = await assetRepository.list(projectId);
      loadedProjectId = projectId;
      state.selectedId = state.items[0]?.id ?? null;
      state.notice = state.items.length ? "" : "当前项目还没有素材";
    } catch (error) {
      state.notice = error instanceof Error ? error.message : String(error);
    } finally {
      loadingPromise = undefined;
    }
  })();
  return loadingPromise;
}

async function importPaths(paths: string[]): Promise<number> {
  const projectId = activeProjectId();
  if (!projectId) {
    state.notice = "请先在项目页打开一个项目";
    return 0;
  }
  if (!paths.length) return 0;
  try {
    const imported = await assetRepository.importPaths(projectId, paths);
    state.items.unshift(...imported);
    if (imported[0]) state.selectedId = imported[0].id;
    state.notice = `已导入并保存 ${imported.length} 个素材`;
    return imported.length;
  } catch (error) {
    state.notice = error instanceof Error ? error.message : String(error);
    return 0;
  }
}

async function importFiles(files: File[], source: ImportSource): Promise<number> {
  const accepted = acceptedFiles(files);
  if (isNativeRuntime()) {
    const projectId = activeProjectId();
    if (!projectId) {
      state.notice = "请先在项目页打开一个项目";
      return 0;
    }
    const imported: AssetItem[] = [];
    try {
      for (const file of accepted) {
        if (file.size > 20 * 1024 * 1024) {
          throw new Error(`“${file.name}”超过剪贴板导入的 20 MB 上限，请使用上传素材`);
        }
        imported.push(await assetRepository.importPayload(
          projectId,
          file.name,
          await fileAsBase64(file),
          source,
        ));
      }
      state.items.unshift(...imported);
      if (imported[0]) state.selectedId = imported[0].id;
      const ignored = files.length - accepted.length;
      state.notice = imported.length
        ? `已导入并保存 ${imported.length} 个素材${ignored ? `，忽略 ${ignored} 个不支持的文件` : ""}`
        : "未发现支持的图片、音频或视频文件";
      return imported.length;
    } catch (error) {
      state.notice = error instanceof Error ? error.message : String(error);
      await loadActiveProject(true);
      return 0;
    }
  }
  const imported = await Promise.all(accepted.map(async (file) => {
    const nextVersion = await versionFromFile(file, 1, "初始版本");
    const asset: AssetItem = {
      id: uid("asset"),
      name: stripFileExtension(file.name),
      category: nextVersion.mediaType === "audio" ? "音频" : "场景",
      mediaType: nextVersion.mediaType,
      source,
      description: "新导入的项目素材，可在右侧补充用途和说明。",
      fallbackImage: nextVersion.mediaType === "audio" ? "audio" : "clouds",
      currentVersionId: nextVersion.id,
      versions: [nextVersion],
      linkedShotIds: [],
    };
    return asset;
  }));
  state.items.unshift(...imported);
  if (imported[0]) {
    state.selectedId = imported[0].id;
    state.activeCategory = "全部";
  }
  const ignored = files.length - accepted.length;
  state.notice = imported.length
    ? `已导入 ${imported.length} 个素材${ignored ? `，忽略 ${ignored} 个不支持的文件` : ""}`
    : "未发现支持的图片或音频文件";
  return imported.length;
}

async function replaceSelected(file: File): Promise<boolean> {
  const asset = selectedAsset.value;
  if (!asset || !acceptedFiles([file]).length) {
    state.notice = "请选择图片或音频文件";
    return false;
  }
  const next = await versionFromFile(file, asset.versions.length + 1, "替换文件，保留历史版本");
  if (next.mediaType !== asset.mediaType) {
    URL.revokeObjectURL(next.previewUrl ?? "");
    state.notice = `替换文件必须保持为${asset.mediaType === "image" ? "图片" : "音频"}`;
    return false;
  }
  const { mediaType: _, ...assetVersion } = next;
  asset.versions.unshift(assetVersion);
  asset.currentVersionId = assetVersion.id;
  state.notice = `${asset.name} 已新增 ${assetVersion.label}，旧版本仍可查看`;
  return true;
}

async function replaceSelectedPath(sourcePath: string): Promise<boolean> {
  const asset = selectedAsset.value;
  if (!asset) return false;
  try {
    const saved = await assetRepository.replace(asset.id, sourcePath);
    replaceAsset(saved);
    state.notice = `${saved.name} 已新增 ${currentAssetVersion(saved).label}，旧版本仍可查看`;
    return true;
  } catch (error) {
    state.notice = error instanceof Error ? error.message : String(error);
    return false;
  }
}

function replaceAsset(saved: AssetItem) {
  const index = state.items.findIndex((item) => item.id === saved.id);
  if (index >= 0) state.items[index] = saved;
}

async function persistMetadata(asset: AssetItem) {
  if (!isNativeRuntime()) return;
  try {
    replaceAsset(await assetRepository.update({
      id: asset.id,
      name: asset.name,
      category: asset.category,
      description: asset.description,
    }));
    state.notice = "素材信息已保存";
  } catch (error) {
    state.notice = error instanceof Error ? error.message : String(error);
    await loadActiveProject(true);
  }
}

async function renameSelected(name: string) {
  const asset = selectedAsset.value;
  const nextName = name.trim();
  if (asset && nextName) {
    asset.name = nextName;
    await persistMetadata(asset);
  }
}

async function updateDescription(description: string) {
  const asset = selectedAsset.value;
  if (asset) {
    asset.description = description;
    await persistMetadata(asset);
  }
}

async function changeCategory(category: AssetCategory) {
  const asset = selectedAsset.value;
  if (!asset || (asset.mediaType === "audio" && category !== "音频") || (asset.mediaType !== "audio" && category === "音频")) return;
  asset.category = category;
  await persistMetadata(asset);
}

async function activateVersion(versionId: string) {
  const asset = selectedAsset.value;
  if (!asset?.versions.some((item) => item.id === versionId)) return;
  if (!isNativeRuntime()) {
    asset.currentVersionId = versionId;
    return;
  }
  try {
    replaceAsset(await assetRepository.setCurrentVersion(asset.id, versionId));
    state.notice = "当前素材版本已切换";
  } catch (error) {
    state.notice = error instanceof Error ? error.message : String(error);
  }
}

async function unlinkShot(shotId: string) {
  const asset = selectedAsset.value;
  if (!asset) return;
  try {
    if (isNativeRuntime()) replaceAsset(await assetRepository.unlink(asset.id, shotId));
    else asset.linkedShotIds = asset.linkedShotIds.filter((id) => id !== shotId);
    state.notice = `已解除与分镜 ${shotId} 的关联`;
  } catch (error) {
    state.notice = error instanceof Error ? error.message : String(error);
  }
}

async function unlinkAll() {
  const asset = selectedAsset.value;
  if (!asset) return;
  try {
    if (isNativeRuntime()) replaceAsset(await assetRepository.unlink(asset.id));
    else asset.linkedShotIds = [];
    state.notice = "已解除该素材的全部分镜关联";
  } catch (error) {
    state.notice = error instanceof Error ? error.message : String(error);
  }
}

async function removeSelected(): Promise<boolean> {
  const asset = selectedAsset.value;
  if (!asset || asset.linkedShotIds.length) {
    state.notice = "素材仍被分镜使用，需先解除关联";
    return false;
  }
  try {
    if (isNativeRuntime()) await assetRepository.delete(asset.id);
    const index = state.items.findIndex((item) => item.id === asset.id);
    state.items.splice(index, 1);
    state.selectedId = state.items[0]?.id ?? null;
    state.notice = "未使用素材已删除";
    return true;
  } catch (error) {
    state.notice = error instanceof Error ? error.message : String(error);
    return false;
  }
}

export function useAssetStore() {
  if (isNativeRuntime() && activeProjectId() !== loadedProjectId) void loadActiveProject();
  return {
    state,
    selectedAsset,
    filteredAssets,
    currentVersion: computed(() => selectedAsset.value ? currentAssetVersion(selectedAsset.value) : null),
    selectAsset: (id: string) => { state.selectedId = id; },
    importFiles,
    importPaths,
    loadActiveProject,
    replaceSelected,
    replaceSelectedPath,
    renameSelected,
    updateDescription,
    changeCategory,
    activateVersion,
    unlinkShot,
    unlinkAll,
    removeSelected,
  };
}
