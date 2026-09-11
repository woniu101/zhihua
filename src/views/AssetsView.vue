<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { ChevronDown, FileAudio2, ImagePlus, Link2, LoaderCircle, MoreVertical, Pencil, Search, Sparkles, Trash2, Upload, X } from "lucide-vue-next";
import { ASSET_CATEGORIES, currentAssetVersion, type AssetCategory, type AssetItem } from "../domain/assets";
import type { GenerationJob } from "../domain/providers";
import { frameProfile } from "../domain/frameProfiles";
import {
  frameCompositionRepository,
  type FrameBackgroundMode,
  type FrameComposition,
  type FrameFitMode,
} from "../services/frameCompositionRepository";
import { activeProjectId } from "../services/storyboardRepository";
import { useAssetStore } from "../stores/assets";
import { useStoryboardStore } from "../stores/storyboard";
import { isNativeRuntime } from "../services/nativeBridge";
import { assetRepository } from "../services/assetRepository";
import { ComfyUiQwenImageProvider, normalizeConnectionFailure, serviceRepository } from "../services/serviceRepository";

const store = useAssetStore();
const storyboard = useStoryboardStore();
const uploadInput = ref<HTMLInputElement | null>(null);
const replaceInput = ref<HTMLInputElement | null>(null);
const dragActive = ref(false);
const composition = ref<FrameComposition>();
const fitMode = ref<FrameFitMode>("cover");
const focalX = ref(0.5);
const focalY = ref(0.5);
const backgroundMode = ref<FrameBackgroundMode>("edge");
const compositionBusy = ref(false);
const compositionNotice = ref("");
const imagePanelOpen = ref(false);
const imageMode = ref<"generate" | "edit">("generate");
const imagePrompt = ref("");
const imageJob = ref<GenerationJob>();
const imageBusy = ref(false);
const imageNotice = ref("");
const imageProvider = new ComfyUiQwenImageProvider();
const selected = store.selectedAsset;
const selectedVersion = store.currentVersion;
const activeProfile = computed(() => frameProfile(storyboard.settings.value.aspectRatio));
const selectedVersionIndex = computed(() => {
  if (!selected.value) return 0;
  return selected.value.versions.findIndex((item) => item.id === selected.value?.currentVersionId) + 1;
});
const replacementAccept = computed(() => selected.value?.mediaType === "audio" ? "audio/*" : selected.value?.mediaType === "video" ? "video/*" : "image/*");
let unlistenDragDrop: UnlistenFn | undefined;
let imagePollTimer: number | undefined;

const imageProgress = computed(() => Math.round((imageJob.value?.progress ?? 0) * 100));

function openImagePanel(mode: "generate" | "edit") {
  if (mode === "edit" && selected.value?.mediaType !== "image") return;
  imageMode.value = mode;
  imagePrompt.value = mode === "generate"
    ? storyboard.selectedScene.value?.visualPlan ?? ""
    : "";
  imageJob.value = undefined;
  imageNotice.value = mode === "generate"
    ? "描述想要的关键画面，结果会自动保存到当前项目素材库。"
    : `将以“${selected.value?.name ?? "当前图片"}”的项目画幅版本为基础编辑。`;
  imagePanelOpen.value = true;
}

async function pollImageJob(jobId: string) {
  try {
    const job = await imageProvider.getStatus(jobId);
    imageJob.value = job;
    if (job.status === "completed") {
      const projectId = activeProjectId();
      if (!projectId) throw new Error("当前项目已关闭，无法保存生成图片。");
      const imported = await assetRepository.downloadCompletedImage(projectId, job.id);
      await store.loadActiveProject(true);
      const asset = imported[0];
      if (asset) {
        store.selectAsset(asset.id);
        const version = currentAssetVersion(asset);
        await frameCompositionRepository.save({
          projectId,
          assetId: asset.id,
          assetVersionId: version.id,
          aspectRatio: activeProfile.value.aspectRatio,
          fitMode: "cover",
          focalX: 0.5,
          focalY: 0.5,
          backgroundMode: "edge",
        });
        const scene = storyboard.selectedScene.value;
        if (scene) {
          await storyboard.save(scene.id, {
            assetIds: [...new Set([...scene.assetIds, asset.id])],
            generationMode: "i2v",
          });
        }
      }
      imageNotice.value = imported.length
        ? `图片已保存到素材库，并套用 ${activeProfile.value.aspectRatio} 最终画框。`
        : "任务完成，但没有可保存的图片。";
      imageBusy.value = false;
      return;
    }
    if (["failed", "cancelled", "interrupted"].includes(job.status)) {
      imageNotice.value = job.errorMessage ?? job.stageMessage ?? "图片任务未完成。";
      imageBusy.value = false;
      return;
    }
    imageNotice.value = job.stageMessage || "远端正在生成图片。";
  } catch (error) {
    imageNotice.value = `${normalizeConnectionFailure(error).message}，正在继续恢复远端任务。`;
  }
  imagePollTimer = window.setTimeout(() => void pollImageJob(jobId), 2500);
}

async function submitImage() {
  const projectId = activeProjectId();
  const prompt = imagePrompt.value.trim();
  if (!projectId || !prompt || imageBusy.value) {
    if (!prompt) imageNotice.value = "请先填写图片描述或编辑要求。";
    return;
  }
  if (imageMode.value === "edit" && selected.value?.mediaType !== "image") {
    imageNotice.value = "请选择一张图片素材后再编辑。";
    return;
  }
  imageBusy.value = true;
  imageNotice.value = "正在检查工作流并准备 GPU。";
  if (imagePollTimer !== undefined) window.clearTimeout(imagePollTimer);
  try {
    const job = await imageProvider.submit({
      clientRequestId: `image-${imageMode.value}-${crypto.randomUUID()}`,
      projectId,
      sceneId: storyboard.selectedScene.value?.id ?? "project-assets",
      mode: imageMode.value,
      aspectRatio: activeProfile.value.aspectRatio,
      prompt,
      seed: Math.floor(Math.random() * 2_147_483_647),
      sourceAssetId: imageMode.value === "edit" ? selected.value?.id : undefined,
    });
    imageJob.value = job;
    await pollImageJob(job.id);
  } catch (error) {
    imageNotice.value = normalizeConnectionFailure(error).message;
    imageBusy.value = false;
  }
}

async function cancelImage() {
  const job = imageJob.value;
  if (!job || !imageBusy.value) return;
  imageNotice.value = "正在取消远端图片任务。";
  try {
    await imageProvider.cancel(job.id);
  } catch (error) {
    imageNotice.value = normalizeConnectionFailure(error).message;
  }
}

async function recoverImageJob() {
  const projectId = activeProjectId();
  if (!projectId) return;
  const jobs = await serviceRepository.listLocalJobs(projectId).catch(() => undefined);
  const resumable = jobs
    ?.slice()
    .reverse()
    .find((item) =>
      ["image_generation", "image_edit"].includes(item.kind)
      && Boolean(item.remoteJobId)
      && item.status !== "completed_local"
      && !["failed", "cancelled", "interrupted"].includes(item.status),
    );
  if (!resumable?.remoteJobId) return;
  imageMode.value = resumable.kind === "image_edit" ? "edit" : "generate";
  imagePanelOpen.value = true;
  imageBusy.value = true;
  imageNotice.value = "已从本地任务队列恢复图片任务，正在核对远端状态。";
  await pollImageJob(resumable.remoteJobId);
}

async function chooseFiles() {
  if (!isNativeRuntime()) {
    uploadInput.value?.click();
    return;
  }
  const selection = await open({
    multiple: true,
    directory: false,
    filters: [{
      name: "图片、音频与参考视频",
      extensions: ["jpg", "jpeg", "png", "webp", "mp3", "wav", "mp4", "mov", "webm"],
    }],
  });
  const paths = selection ? (Array.isArray(selection) ? selection : [selection]) : [];
  await store.importPaths(paths);
}

async function chooseReplacement() {
  if (!selected.value) return;
  if (!isNativeRuntime()) {
    replaceInput.value?.click();
    return;
  }
  const extensions = selected.value.mediaType === "image"
    ? ["jpg", "jpeg", "png", "webp"]
    : selected.value.mediaType === "audio"
      ? ["mp3", "wav"]
      : ["mp4", "mov", "webm"];
  const selection = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "同类型素材", extensions }],
  });
  if (typeof selection === "string") await store.replaceSelectedPath(selection);
}

function previewStyle(asset: AssetItem) {
  const previewUrl = currentAssetVersion(asset).previewUrl;
  return previewUrl ? { backgroundImage: `url(${JSON.stringify(previewUrl)})` } : undefined;
}
function currentPreviewStyle() {
  const previewUrl = selectedVersion.value?.previewUrl;
  if (!previewUrl) return undefined;
  return {
    backgroundImage: `url(${JSON.stringify(previewUrl)})`,
    backgroundSize: fitMode.value,
    backgroundPosition: `${Math.round(focalX.value * 100)}% ${Math.round(focalY.value * 100)}%`,
    backgroundRepeat: "no-repeat",
  };
}
function framePreviewStyle() {
  return {
    ...currentPreviewStyle(),
    aspectRatio: `${activeProfile.value.visibleWidth} / ${activeProfile.value.visibleHeight}`,
  };
}

async function loadComposition() {
  const projectId = activeProjectId();
  const asset = selected.value;
  const version = selectedVersion.value;
  if (!projectId || !asset || !version || asset.mediaType !== "image") {
    composition.value = undefined;
    if (store.state.detailTab === "frame") store.state.detailTab = "versions";
    return;
  }
  const saved = await frameCompositionRepository.get({
    projectId,
    assetId: asset.id,
    assetVersionId: version.id,
    aspectRatio: storyboard.settings.value.aspectRatio,
  }).catch(() => undefined);
  composition.value = saved ?? undefined;
  fitMode.value = saved?.fitMode ?? "cover";
  focalX.value = saved?.focalX ?? 0.5;
  focalY.value = saved?.focalY ?? 0.5;
  backgroundMode.value = saved?.backgroundMode ?? "edge";
  compositionNotice.value = saved ? "已载入该版本的项目画幅构图" : "尚未保存该画幅构图";
}

async function saveComposition() {
  const projectId = activeProjectId();
  const asset = selected.value;
  const version = selectedVersion.value;
  if (!projectId || !asset || !version || asset.mediaType !== "image") return;
  compositionBusy.value = true;
  compositionNotice.value = "正在保存画幅构图";
  try {
    const saved = await frameCompositionRepository.save({
      projectId,
      assetId: asset.id,
      assetVersionId: version.id,
      aspectRatio: storyboard.settings.value.aspectRatio,
      fitMode: fitMode.value,
      focalX: focalX.value,
      focalY: focalY.value,
      backgroundMode: backgroundMode.value,
    });
    if (!saved) throw new Error("画幅构图只能在桌面客户端中保存");
    composition.value = saved;
    compositionNotice.value = `${saved.aspectRatio} 构图已保存，不会修改原图`;
  } catch (error) {
    compositionNotice.value = error instanceof Error ? error.message : String(error);
  } finally {
    compositionBusy.value = false;
  }
}
async function importSelection(event: Event, source: "本地上传" | "剪贴板" = "本地上传") {
  const input = event.target as HTMLInputElement;
  await store.importFiles(Array.from(input.files ?? []), source);
  input.value = "";
}
async function replaceSelection(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (file) await store.replaceSelected(file);
  input.value = "";
}
async function onDrop(event: DragEvent) {
  dragActive.value = false;
  await store.importFiles(Array.from(event.dataTransfer?.files ?? []), "本地上传");
}
async function readClipboard() {
  try {
    const clipboard = navigator.clipboard as Clipboard & { read?: () => Promise<ClipboardItems> };
    if (!clipboard.read) throw new Error("unavailable");
    const files: File[] = [];
    const items = await clipboard.read();
    for (const item of items) {
      const mediaType = item.types.find((type) => type.startsWith("image/") || type.startsWith("audio/"));
      if (!mediaType) continue;
      const blob = await item.getType(mediaType);
      const extension = mediaType.split("/")[1]?.replace("jpeg", "jpg") || "bin";
      files.push(new File([blob], `剪贴板素材-${Date.now()}.${extension}`, { type: mediaType }));
    }
    await store.importFiles(files, "剪贴板");
  } catch {
    store.state.notice = "未能直接读取剪贴板，请复制图片后按 Ctrl+V 导入";
  }
}
async function onPaste(event: ClipboardEvent) {
  const files = Array.from(event.clipboardData?.items ?? []).map((item) => item.getAsFile()).filter((file): file is File => Boolean(file));
  if (files.length) await store.importFiles(files, "剪贴板");
}
function setCategory(event: Event) {
  store.changeCategory((event.target as HTMLSelectElement).value as AssetCategory);
}
watch(
  [
    () => selected.value?.id,
    () => selectedVersion.value?.id,
    () => storyboard.settings.value.aspectRatio,
  ],
  () => void loadComposition(),
  { immediate: true },
);
onMounted(async () => {
  window.addEventListener("paste", onPaste);
  await store.loadActiveProject();
  void recoverImageJob();
  if (isNativeRuntime()) {
    void getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "enter" || event.payload.type === "over") {
        dragActive.value = true;
      } else if (event.payload.type === "drop") {
        dragActive.value = false;
        void store.importPaths(event.payload.paths);
      } else {
        dragActive.value = false;
      }
    }).then((unlisten) => { unlistenDragDrop = unlisten; });
  }
});
onBeforeUnmount(() => {
  window.removeEventListener("paste", onPaste);
  unlistenDragDrop?.();
  if (imagePollTimer !== undefined) window.clearTimeout(imagePollTimer);
});
</script>

<template>
  <section class="page assets-page">
    <header class="page-head">
      <div><p class="breadcrumb">素材　/　素材库</p><div class="page-title-line"><h1>素材库</h1><span class="status-line"><span class="dot blue"></span><strong>优云智算 · 无卡运行</strong><span>|</span><span>生成时再切换 GPU</span></span></div></div>
      <div class="head-actions">
        <div class="status-pill"><span class="dot blue"></span>无卡模式</div>
        <button class="btn primary" @click="chooseFiles"><Upload :size="19"/>上传素材</button>
        <button class="btn" @click="openImagePanel('generate')"><Sparkles :size="18"/>生成图片</button>
        <button class="btn" @click="readClipboard">▣　从剪贴板粘贴</button>
        <label class="search-box"><Search :size="19"/><input v-model="store.state.query" placeholder="搜索素材、分类或描述"/></label>
        <input ref="uploadInput" class="visually-hidden" type="file" multiple accept="image/*,audio/*,video/*" @change="importSelection"/>
      </div>
    </header>

    <div class="asset-tabs" aria-label="素材分类">
      <button v-for="item in ASSET_CATEGORIES" :key="item" class="tab-btn" :class="{active:store.state.activeCategory===item}" @click="store.state.activeCategory=item">{{ item }}</button>
      <span v-if="store.state.notice" class="asset-notice">{{ store.state.notice }}</span>
    </div>

    <div class="asset-layout">
      <section class="asset-library">
        <button class="drop-zone" :class="{dragging:dragActive}" @click="chooseFiles" @dragenter.prevent="dragActive=true" @dragover.prevent="dragActive=true" @dragleave.prevent="dragActive=false" @drop.prevent="onDrop">
          <Upload :size="21"/><b>导入图片、音频或参考视频</b><span>支持 JPG、PNG、WebP、MP3、WAV、MP4、MOV 和 WebM，也可直接按 Ctrl+V</span>
        </button>
        <div v-if="store.filteredAssets.value.length" class="asset-grid">
          <article v-for="asset in store.filteredAssets.value" :key="asset.id" :class="{selected:store.state.selectedId===asset.id}" @click="store.selectAsset(asset.id)">
            <div class="asset-thumb" :class="`asset-${asset.fallbackImage}`" :style="previewStyle(asset)"><FileAudio2 v-if="asset.mediaType==='audio'" class="audio-mark" :size="31"/><span v-if="store.state.selectedId===asset.id" class="selected-mark">✓</span></div>
            <div class="asset-name"><h3>{{ asset.name }} <span>{{ asset.category }}</span></h3><button aria-label="选择素材详情" @click.stop="store.selectAsset(asset.id)"><MoreVertical :size="18"/></button></div>
            <footer><span>{{ currentAssetVersion(asset).dimensions ?? currentAssetVersion(asset).duration ?? currentAssetVersion(asset).format }}　| {{ currentAssetVersion(asset).label }}</span><a><Link2 :size="15"/>{{ asset.linkedShotIds.length }} 个分镜</a></footer>
          </article>
        </div>
        <div v-else class="empty-assets"><Search :size="28"/><b>没有匹配的素材</b><span>更换分类或搜索关键词后再试</span></div>
        <div class="style-profile panel"><div class="style-head"><h3><ChevronDown :size="18"/>项目视觉规范</h3><a>在项目设置中编辑　›</a></div><div class="style-items"><div><small>风格预设</small><span class="mini-img asset-bolt"></span><b>科普写实</b></div><div><small>主色板</small><span class="colors">● ● ● ● ●</span><b>蓝色为主</b></div><div><small>线条与材质</small><span class="mini-img asset-clouds"></span><b>清晰轮廓</b></div><div><small>光线</small><span class="mini-img asset-bolt"></span><b>高对比</b></div><div><small>背景复杂度</small><span class="mini-img asset-mountain"></span><b>中等</b></div><div><small>禁止元素</small><span class="ban">⊗</span><b>血腥、暴力</b></div></div></div>
      </section>

      <aside v-if="selected && selectedVersion" class="panel asset-detail">
        <div class="panel-head"><h2>素材详情</h2><button aria-label="关闭详情" @click="store.state.selectedId=null"><X :size="20"/></button></div>
        <div class="detail-preview" :class="`asset-${selected.fallbackImage}`" :style="currentPreviewStyle()"><FileAudio2 v-if="selected.mediaType==='audio'" class="detail-audio" :size="54"/><span>{{ selectedVersionIndex }} / {{ selected.versions.length }}</span></div>
        <div class="detail-form">
          <label>名称 <input :value="selected.name" @change="store.renameSelected(($event.target as HTMLInputElement).value)"/></label>
          <label>分类 <select :value="selected.category" @change="setCategory"><option v-for="category in ASSET_CATEGORIES.slice(1)" :key="category" :value="category" :disabled="selected.mediaType==='audio' ? category!=='音频' : category==='音频'">{{ category }}</option></select></label>
          <p><span>来源</span>{{ selected.source }}</p><p><span>规格</span>{{ selectedVersion.dimensions ?? selectedVersion.duration ?? '待读取' }} · {{ selectedVersion.format }}</p><p><span>版本</span>{{ selectedVersion.label }}　|　共 {{ selected.versions.length }} 个历史版本</p><p><span>导入时间</span>{{ selectedVersion.createdAt }}</p>
          <label>描述 <textarea :value="selected.description" @change="store.updateDescription(($event.target as HTMLTextAreaElement).value)"></textarea></label>
        </div>
        <div class="detail-tabs"><button :class="{active:store.state.detailTab==='versions'}" @click="store.state.detailTab='versions'">版本记录 ({{ selected.versions.length }})</button><button :class="{active:store.state.detailTab==='links'}" @click="store.state.detailTab='links'">关联分镜 ({{ selected.linkedShotIds.length }})</button><button v-if="selected.mediaType==='image'" :class="{active:store.state.detailTab==='frame'}" @click="store.state.detailTab='frame'">画幅适配</button></div>
        <div v-if="store.state.detailTab==='versions'" class="version-list">
          <article v-for="version in selected.versions" :key="version.id" :class="{selected:version.id===selected.currentVersionId}" @click="store.activateVersion(version.id)"><span class="version-thumb" :class="`asset-${selected.fallbackImage}`" :style="version.previewUrl ? {backgroundImage:`url(${JSON.stringify(version.previewUrl)})`} : undefined"></span><div><b>{{ version.label }}</b><small>{{ version.createdAt }}</small><p>{{ version.note }}</p></div><em v-if="version.id===selected.currentVersionId">当前版本</em><button v-else>设为当前</button></article>
        </div>
        <div v-else-if="store.state.detailTab==='links'" class="link-list">
          <article v-for="shotId in selected.linkedShotIds" :key="shotId"><span class="link-badge">{{ shotId }}</span><div><b>分镜 {{ shotId }}</b><small>使用当前素材版本</small></div><button class="btn link" @click="store.unlinkShot(shotId)">解除关联</button></article>
          <div v-if="!selected.linkedShotIds.length" class="empty-links"><Link2 :size="25"/><b>暂未关联分镜</b><span>可在分镜页将本素材绑定为参考输入</span></div>
        </div>
        <div v-else class="frame-editor">
          <div class="frame-preview" :style="framePreviewStyle()"><span>{{ activeProfile.aspectRatio }} · 最终可见画框</span></div>
          <div class="frame-choice"><button :class="{active:fitMode==='cover'}" @click="fitMode='cover'">裁切填满</button><button :class="{active:fitMode==='contain'}" @click="fitMode='contain'">完整显示</button></div>
          <label><span>主体水平位置</span><input v-model.number="focalX" type="range" min="0" max="1" step="0.01"/></label>
          <label><span>主体垂直位置</span><input v-model.number="focalY" type="range" min="0" max="1" step="0.01"/></label>
          <label v-if="fitMode==='contain'"><span>留白处理</span><select v-model="backgroundMode"><option value="edge">延展边缘</option><option value="blur">模糊背景</option><option value="solid">纯色背景</option></select></label>
          <p>{{ activeProfile.visibleWidth }}×{{ activeProfile.visibleHeight }} 可见画面 · {{ activeProfile.workWidth }}×{{ activeProfile.workHeight }} H3 工作画布</p>
          <button class="btn primary" :disabled="compositionBusy" @click="saveComposition">{{ compositionBusy ? '正在保存' : '保存该画幅构图' }}</button>
          <small>{{ compositionNotice }}</small>
        </div>
        <footer><button v-if="selected.mediaType==='image'" class="btn primary" @click="openImagePanel('edit')"><Pencil :size="16"/>AI 编辑图片</button><button v-else class="btn primary" @click="chooseReplacement">▣　替换文件</button><button v-if="selected.linkedShotIds.length" class="btn" @click="store.unlinkAll">解除全部关联</button><button v-else class="btn danger" @click="store.removeSelected"><Trash2 :size="16"/>删除素材</button><input ref="replaceInput" class="visually-hidden" type="file" :accept="replacementAccept" @change="replaceSelection"/></footer>
      </aside>
      <aside v-else class="panel asset-detail detail-empty"><Upload :size="34"/><b>选择一个素材查看详情</b></aside>
    </div>
    <div v-if="imagePanelOpen" class="image-modal-backdrop" @click.self="!imageBusy && (imagePanelOpen=false)">
      <section class="image-modal panel" role="dialog" aria-modal="true" aria-label="AI 图片生成与编辑">
        <header><div class="image-modal-title"><span><ImagePlus :size="22"/></span><div><h2>{{ imageMode==='generate' ? '生成分镜关键画面' : '编辑当前图片' }}</h2><p>{{ activeProfile.aspectRatio }} · {{ activeProfile.workWidth }}×{{ activeProfile.workHeight }} 工作图 · 自动保存版本</p></div></div><button :disabled="imageBusy" aria-label="关闭" @click="imagePanelOpen=false"><X :size="20"/></button></header>
        <div class="image-mode-tabs"><button :class="{active:imageMode==='generate'}" :disabled="imageBusy" @click="imageMode='generate'">从描述生成</button><button :class="{active:imageMode==='edit'}" :disabled="imageBusy || selected?.mediaType!=='image'" @click="imageMode='edit'">编辑所选图片</button></div>
        <label><span>{{ imageMode==='generate' ? '画面描述' : '编辑要求' }}</span><textarea v-model="imagePrompt" :disabled="imageBusy" :placeholder="imageMode==='generate' ? '例如：深蓝雷云覆盖群山，一道闪电连接云层与地面，科普插画，清晰轮廓，无文字' : '例如：保持主体和构图不变，把夜空调整为雨后的蓝紫色，并增强闪电亮度'"/></label>
        <div v-if="imageMode==='edit'" class="edit-source"><div class="edit-source-thumb" :style="selected ? previewStyle(selected) : undefined"></div><div><b>{{ selected?.name }}</b><span>使用已确认的 {{ activeProfile.aspectRatio }} 构图作为编辑输入</span></div></div>
        <div class="image-task"><div class="field-head"><b>{{ imageJob ? `任务 ${imageJob.id.slice(0,8)}` : '提交前保持无卡模式' }}</b><span>{{ imageJob ? `${imageProgress}%` : '0%' }}</span></div><div class="progress"><i :style="{width:`${imageProgress}%`}"></i></div><p>{{ imageNotice }}</p></div>
        <footer><button v-if="imageBusy && imageJob" class="btn danger" @click="cancelImage">取消任务</button><button class="btn" :disabled="imageBusy" @click="imagePanelOpen=false">关闭</button><button class="btn primary" :disabled="imageBusy || !imagePrompt.trim()" @click="submitImage"><LoaderCircle v-if="imageBusy" class="spin" :size="17"/><Sparkles v-else :size="17"/>{{ imageBusy ? '生成中' : imageMode==='generate' ? '生成图片' : '生成编辑版本' }}</button></footer>
      </section>
    </div>
  </section>
</template>

<style scoped>
.assets-page{display:grid;grid-template-rows:90px 46px minmax(0,1fr);gap:8px}.breadcrumb{color:#576b8f;margin:0 0 6px 4px}.status-pill{height:42px;border:1px solid var(--line);border-radius:8px;background:#fff;display:flex;align-items:center;gap:8px;padding:0 14px;font-weight:650}.search-box{min-width:220px}.visually-hidden{position:fixed;left:-10000px;width:1px;height:1px;opacity:0}.asset-tabs{display:flex;align-items:center;gap:9px}.asset-tabs .tab-btn{min-width:78px}.asset-notice{margin-left:auto;max-width:360px;color:#526b91;font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.asset-layout{min-height:0;display:grid;grid-template-columns:minmax(620px,1fr) 426px;gap:15px}.asset-library{min-height:0;display:flex;flex-direction:column;gap:10px}.drop-zone{height:80px;border:1.5px dashed #5a9dff;border-radius:9px;background:transparent;display:flex;flex-direction:column;align-items:center;justify-content:center;color:var(--blue);gap:3px}.drop-zone.dragging{background:#eaf3ff;border-style:solid}.drop-zone b{font-size:14px}.drop-zone span{font-size:12px;color:#7184a3}.asset-grid{flex:1;min-height:0;display:grid;grid-template-columns:repeat(3,1fr);grid-auto-rows:minmax(130px,1fr);gap:11px;overflow:auto;padding:1px}.asset-grid article{min-height:130px;border:1px solid var(--line);border-radius:8px;background:#fff;overflow:hidden;box-shadow:var(--shadow);display:grid;grid-template-rows:minmax(70px,1fr) 31px 29px;cursor:pointer}.asset-grid article.selected{border:2px solid var(--blue)}.asset-thumb,.detail-preview,.version-thumb,.mini-img{background-position:center;background-size:cover}.asset-clouds{background-image:url('../assets/asset-clouds.jpg')}.asset-bolt{background-image:url('../assets/asset-bolt.jpg')}.asset-runner{background-image:url('../assets/asset-runner.jpg')}.asset-mountain{background-image:url('../assets/asset-mountain.jpg')}.asset-palette{background-image:url('../assets/asset-palette.jpg')}.asset-audio{background-image:url('../assets/asset-audio.jpg')}.asset-safety{background-image:url('../assets/asset-safety.jpg')}.asset-village{background-image:url('../assets/asset-village.jpg')}.asset-street{background-image:url('../assets/asset-street.jpg')}.asset-thumb{position:relative}.audio-mark{position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);color:#fff;filter:drop-shadow(0 2px 4px #18355e)}.selected-mark{position:absolute;right:8px;top:8px;width:25px;height:25px;border-radius:50%;display:grid;place-items:center;color:#fff;background:var(--blue)}.asset-name{padding:3px 10px;display:flex;align-items:center;justify-content:space-between;min-width:0}.asset-name h3{font-size:14px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.asset-name h3 span{font-size:11px;color:var(--blue);background:#e9f3ff;padding:3px 6px;border-radius:4px}.asset-name button{border:0;background:transparent}.asset-grid article footer{padding:0 10px;display:flex;align-items:center;justify-content:space-between;font-size:11px;color:#64789a}.asset-grid footer a{display:flex;align-items:center;gap:4px;color:var(--blue)}.empty-assets{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:7px;color:#7184a3}.style-profile{height:170px;padding:9px 11px}.style-head{display:flex;align-items:center;justify-content:space-between;height:30px}.style-head h3{display:flex;align-items:center}.style-head a{color:var(--blue);font-size:12px}.style-items{display:grid;grid-template-columns:repeat(6,1fr);gap:8px}.style-items>div{height:111px;border:1px solid #dfe7f2;border-radius:7px;padding:8px;display:flex;flex-direction:column;gap:5px}.style-items small{color:#354c72}.style-items b{font-size:10px;color:#617595}.mini-img{height:46px;border-radius:5px}.colors{font-size:23px;white-space:nowrap;color:#0f5aa8}.ban{height:46px;display:grid;place-items:center;color:#f44336;font-size:38px}.asset-detail{min-height:0;display:flex;flex-direction:column;overflow:hidden}.asset-detail .panel-head button{border:0;background:transparent}.detail-preview{height:198px;margin:0 15px;position:relative;border-radius:8px}.detail-preview>span{position:absolute;right:8px;bottom:8px;color:#fff;background:#071a38;padding:4px 7px;border-radius:4px;font-size:11px}.detail-audio{position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);color:#fff}.detail-form{padding:10px 16px;display:flex;flex-direction:column;gap:7px}.detail-form label{display:grid;grid-template-columns:72px 1fr;align-items:start;font-size:12px}.detail-form input,.detail-form select,.detail-form textarea{border:1px solid #d5e0ef;border-radius:6px;background:#fff;min-height:34px;padding:0 9px}.detail-form p{font-size:12px}.detail-form p span{display:inline-block;width:72px;color:#5f7396}.detail-form textarea{height:70px;resize:none;padding:8px;line-height:1.5}.detail-tabs{height:42px;border-bottom:1px solid var(--line);display:flex;padding:0 15px;gap:30px}.detail-tabs button{border:0;background:transparent;position:relative;font-weight:700}.detail-tabs .active{color:var(--blue)}.detail-tabs .active:after{content:"";position:absolute;bottom:0;left:0;right:0;height:3px;background:var(--blue)}.version-list,.link-list{padding:8px 15px;display:flex;flex-direction:column;gap:8px;overflow:auto}.version-list article{min-height:64px;border:1px solid #dbe4f0;border-radius:7px;display:flex;align-items:center;gap:10px;padding:6px;cursor:pointer}.version-list article.selected{border-color:var(--blue);background:#f3f7ff}.version-thumb{width:82px;height:48px;border-radius:5px;flex:0 0 auto}.version-list small{margin-left:8px;color:#6c80a1}.version-list p{font-size:11px;color:#637797;margin-top:4px}.version-list em{margin-left:auto;font-style:normal;color:var(--blue);font-size:11px}.version-list article>button{margin-left:auto;border:0;background:transparent;color:var(--blue);font-size:11px}.link-list article{height:58px;border:1px solid #dbe4f0;border-radius:7px;display:flex;align-items:center;gap:10px;padding:7px}.link-badge{width:37px;height:37px;display:grid;place-items:center;border-radius:7px;background:#eaf3ff;color:var(--blue);font-weight:700}.link-list article div{display:flex;flex-direction:column;gap:3px}.link-list small{font-size:11px;color:#697d9c}.link-list article button{margin-left:auto}.empty-links,.detail-empty{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:7px;color:#7184a3}.empty-links span{font-size:12px}.asset-detail>footer{margin-top:auto;border-top:1px solid var(--line);padding:13px 15px;display:grid;grid-template-columns:1fr 1fr;gap:10px}@media(max-width:1380px){.asset-layout{grid-template-columns:minmax(600px,1fr) 380px}.style-profile{height:145px}.style-items>div{height:88px}.mini-img,.ban{height:31px}.asset-grid{gap:8px}.detail-preview{height:155px}.head-actions{gap:6px}.status-pill{display:none}}
.detail-tabs{gap:18px}.detail-tabs button{white-space:nowrap}.frame-editor{padding:10px 15px;display:flex;flex-direction:column;gap:9px;overflow:auto}.frame-preview{width:100%;max-height:150px;min-height:100px;margin:auto;border-radius:7px;background-color:#e7edf5;background-position:center;background-repeat:no-repeat;background-size:cover;position:relative}.frame-preview>span{position:absolute;right:7px;bottom:7px;padding:4px 7px;border-radius:4px;background:rgba(5,20,43,.78);color:#fff;font-size:11px}.frame-choice{display:grid;grid-template-columns:1fr 1fr;gap:8px}.frame-choice button{height:34px;border:1px solid #cad8ea;border-radius:6px;background:#fff}.frame-choice button.active{border-color:var(--blue);background:#edf5ff;color:var(--blue);font-weight:700}.frame-editor label{display:grid;grid-template-columns:105px 1fr;align-items:center;gap:9px;font-size:12px}.frame-editor select{height:32px;border:1px solid #cad8ea;border-radius:6px;background:#fff;padding:0 7px}.frame-editor input[type=range]{accent-color:var(--blue)}.frame-editor p,.frame-editor small{font-size:11px;color:#667b9c}.frame-editor small{min-height:16px}@media(max-width:1380px){.frame-preview{max-height:110px;min-height:80px}}
.image-modal-backdrop{position:fixed;inset:0;z-index:100;background:rgba(13,30,55,.36);display:grid;place-items:center;padding:30px}.image-modal{width:min(640px,calc(100vw - 60px));padding:0;overflow:hidden;box-shadow:0 24px 70px rgba(18,46,83,.25)}.image-modal>header{height:78px;padding:0 22px;border-bottom:1px solid var(--line);display:flex;align-items:center;justify-content:space-between}.image-modal>header>button{border:0;background:transparent}.image-modal-title{display:flex;align-items:center;gap:12px}.image-modal-title>span{width:42px;height:42px;border-radius:10px;background:#eaf3ff;color:var(--blue);display:grid;place-items:center}.image-modal h2{font-size:20px}.image-modal header p{margin-top:4px;color:#6c7e9d;font-size:12px}.image-mode-tabs{margin:18px 22px 0;display:grid;grid-template-columns:1fr 1fr;padding:4px;background:#eef3f9;border-radius:9px}.image-mode-tabs button{height:38px;border:0;border-radius:7px;background:transparent;color:#647797}.image-mode-tabs button.active{background:#fff;color:var(--blue);font-weight:750;box-shadow:0 2px 8px rgba(34,79,139,.1)}.image-modal>label{display:flex;flex-direction:column;gap:8px;margin:18px 22px}.image-modal>label>span{font-weight:700}.image-modal textarea{height:122px;border:1px solid #cad8e9;border-radius:8px;padding:12px;resize:none;line-height:1.6;font:inherit}.edit-source{margin:0 22px 16px;border:1px solid #d9e4f1;border-radius:8px;padding:9px;display:flex;align-items:center;gap:12px;background:#f8fbff}.edit-source-thumb{width:90px;height:58px;border-radius:6px;background-position:center;background-size:cover}.edit-source div:last-child{display:flex;flex-direction:column;gap:5px}.edit-source span{font-size:12px;color:#6a7e9e}.image-task{margin:0 22px 18px;padding:12px;border:1px solid #dce6f2;border-radius:8px;background:#fbfdff}.image-task .field-head{display:flex;justify-content:space-between}.image-task p{margin-top:8px;color:#607596;font-size:12px}.image-modal>footer{height:68px;border-top:1px solid var(--line);display:flex;justify-content:flex-end;align-items:center;gap:10px;padding:0 22px}.spin{animation:spin 1s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}
</style>
