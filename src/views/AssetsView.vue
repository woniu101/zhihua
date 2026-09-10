<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { ChevronDown, FileAudio2, Link2, MoreVertical, Search, Trash2, Upload, X } from "lucide-vue-next";
import { ASSET_CATEGORIES, currentAssetVersion, type AssetCategory, type AssetItem } from "../domain/assets";
import { useAssetStore } from "../stores/assets";
import { isNativeRuntime } from "../services/nativeBridge";

const store = useAssetStore();
const uploadInput = ref<HTMLInputElement | null>(null);
const replaceInput = ref<HTMLInputElement | null>(null);
const dragActive = ref(false);
const selected = store.selectedAsset;
const selectedVersion = store.currentVersion;
const selectedVersionIndex = computed(() => {
  if (!selected.value) return 0;
  return selected.value.versions.findIndex((item) => item.id === selected.value?.currentVersionId) + 1;
});
const replacementAccept = computed(() => selected.value?.mediaType === "audio" ? "audio/*" : selected.value?.mediaType === "video" ? "video/*" : "image/*");
let unlistenDragDrop: UnlistenFn | undefined;

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
  return previewUrl ? { backgroundImage: `url(${JSON.stringify(previewUrl)})` } : undefined;
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
onMounted(() => {
  window.addEventListener("paste", onPaste);
  void store.loadActiveProject();
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
});
</script>

<template>
  <section class="page assets-page">
    <header class="page-head">
      <div><p class="breadcrumb">素材　/　素材库</p><div class="page-title-line"><h1>素材库</h1><span class="status-line"><span class="dot blue"></span><strong>优云智算 · 无卡运行</strong><span>|</span><span>生成时再切换 GPU</span></span></div></div>
      <div class="head-actions">
        <div class="status-pill"><span class="dot blue"></span>无卡模式</div>
        <button class="btn primary" @click="chooseFiles"><Upload :size="19"/>上传素材</button>
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
        <div class="detail-tabs"><button :class="{active:store.state.detailTab==='versions'}" @click="store.state.detailTab='versions'">版本记录 ({{ selected.versions.length }})</button><button :class="{active:store.state.detailTab==='links'}" @click="store.state.detailTab='links'">关联分镜 ({{ selected.linkedShotIds.length }})</button></div>
        <div v-if="store.state.detailTab==='versions'" class="version-list">
          <article v-for="version in selected.versions" :key="version.id" :class="{selected:version.id===selected.currentVersionId}" @click="store.activateVersion(version.id)"><span class="version-thumb" :class="`asset-${selected.fallbackImage}`" :style="version.previewUrl ? {backgroundImage:`url(${JSON.stringify(version.previewUrl)})`} : undefined"></span><div><b>{{ version.label }}</b><small>{{ version.createdAt }}</small><p>{{ version.note }}</p></div><em v-if="version.id===selected.currentVersionId">当前版本</em><button v-else>设为当前</button></article>
        </div>
        <div v-else class="link-list">
          <article v-for="shotId in selected.linkedShotIds" :key="shotId"><span class="link-badge">{{ shotId }}</span><div><b>分镜 {{ shotId }}</b><small>使用当前素材版本</small></div><button class="btn link" @click="store.unlinkShot(shotId)">解除关联</button></article>
          <div v-if="!selected.linkedShotIds.length" class="empty-links"><Link2 :size="25"/><b>暂未关联分镜</b><span>可在分镜页将本素材绑定为参考输入</span></div>
        </div>
        <footer><button class="btn primary" @click="chooseReplacement">▣　替换文件</button><button v-if="selected.linkedShotIds.length" class="btn" @click="store.unlinkAll">解除全部关联</button><button v-else class="btn danger" @click="store.removeSelected"><Trash2 :size="16"/>删除素材</button><input ref="replaceInput" class="visually-hidden" type="file" :accept="replacementAccept" @change="replaceSelection"/></footer>
      </aside>
      <aside v-else class="panel asset-detail detail-empty"><Upload :size="34"/><b>选择一个素材查看详情</b></aside>
    </div>
  </section>
</template>

<style scoped>
.assets-page{display:grid;grid-template-rows:90px 46px minmax(0,1fr);gap:8px}.breadcrumb{color:#576b8f;margin:0 0 6px 4px}.status-pill{height:42px;border:1px solid var(--line);border-radius:8px;background:#fff;display:flex;align-items:center;gap:8px;padding:0 14px;font-weight:650}.search-box{min-width:220px}.visually-hidden{position:fixed;left:-10000px;width:1px;height:1px;opacity:0}.asset-tabs{display:flex;align-items:center;gap:9px}.asset-tabs .tab-btn{min-width:78px}.asset-notice{margin-left:auto;max-width:360px;color:#526b91;font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.asset-layout{min-height:0;display:grid;grid-template-columns:minmax(620px,1fr) 426px;gap:15px}.asset-library{min-height:0;display:flex;flex-direction:column;gap:10px}.drop-zone{height:80px;border:1.5px dashed #5a9dff;border-radius:9px;background:transparent;display:flex;flex-direction:column;align-items:center;justify-content:center;color:var(--blue);gap:3px}.drop-zone.dragging{background:#eaf3ff;border-style:solid}.drop-zone b{font-size:14px}.drop-zone span{font-size:12px;color:#7184a3}.asset-grid{flex:1;min-height:0;display:grid;grid-template-columns:repeat(3,1fr);grid-auto-rows:minmax(130px,1fr);gap:11px;overflow:auto;padding:1px}.asset-grid article{min-height:130px;border:1px solid var(--line);border-radius:8px;background:#fff;overflow:hidden;box-shadow:var(--shadow);display:grid;grid-template-rows:minmax(70px,1fr) 31px 29px;cursor:pointer}.asset-grid article.selected{border:2px solid var(--blue)}.asset-thumb,.detail-preview,.version-thumb,.mini-img{background-position:center;background-size:cover}.asset-clouds{background-image:url('../assets/asset-clouds.jpg')}.asset-bolt{background-image:url('../assets/asset-bolt.jpg')}.asset-runner{background-image:url('../assets/asset-runner.jpg')}.asset-mountain{background-image:url('../assets/asset-mountain.jpg')}.asset-palette{background-image:url('../assets/asset-palette.jpg')}.asset-audio{background-image:url('../assets/asset-audio.jpg')}.asset-safety{background-image:url('../assets/asset-safety.jpg')}.asset-village{background-image:url('../assets/asset-village.jpg')}.asset-street{background-image:url('../assets/asset-street.jpg')}.asset-thumb{position:relative}.audio-mark{position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);color:#fff;filter:drop-shadow(0 2px 4px #18355e)}.selected-mark{position:absolute;right:8px;top:8px;width:25px;height:25px;border-radius:50%;display:grid;place-items:center;color:#fff;background:var(--blue)}.asset-name{padding:3px 10px;display:flex;align-items:center;justify-content:space-between;min-width:0}.asset-name h3{font-size:14px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.asset-name h3 span{font-size:11px;color:var(--blue);background:#e9f3ff;padding:3px 6px;border-radius:4px}.asset-name button{border:0;background:transparent}.asset-grid article footer{padding:0 10px;display:flex;align-items:center;justify-content:space-between;font-size:11px;color:#64789a}.asset-grid footer a{display:flex;align-items:center;gap:4px;color:var(--blue)}.empty-assets{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:7px;color:#7184a3}.style-profile{height:170px;padding:9px 11px}.style-head{display:flex;align-items:center;justify-content:space-between;height:30px}.style-head h3{display:flex;align-items:center}.style-head a{color:var(--blue);font-size:12px}.style-items{display:grid;grid-template-columns:repeat(6,1fr);gap:8px}.style-items>div{height:111px;border:1px solid #dfe7f2;border-radius:7px;padding:8px;display:flex;flex-direction:column;gap:5px}.style-items small{color:#354c72}.style-items b{font-size:10px;color:#617595}.mini-img{height:46px;border-radius:5px}.colors{font-size:23px;white-space:nowrap;color:#0f5aa8}.ban{height:46px;display:grid;place-items:center;color:#f44336;font-size:38px}.asset-detail{min-height:0;display:flex;flex-direction:column;overflow:hidden}.asset-detail .panel-head button{border:0;background:transparent}.detail-preview{height:198px;margin:0 15px;position:relative;border-radius:8px}.detail-preview>span{position:absolute;right:8px;bottom:8px;color:#fff;background:#071a38;padding:4px 7px;border-radius:4px;font-size:11px}.detail-audio{position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);color:#fff}.detail-form{padding:10px 16px;display:flex;flex-direction:column;gap:7px}.detail-form label{display:grid;grid-template-columns:72px 1fr;align-items:start;font-size:12px}.detail-form input,.detail-form select,.detail-form textarea{border:1px solid #d5e0ef;border-radius:6px;background:#fff;min-height:34px;padding:0 9px}.detail-form p{font-size:12px}.detail-form p span{display:inline-block;width:72px;color:#5f7396}.detail-form textarea{height:70px;resize:none;padding:8px;line-height:1.5}.detail-tabs{height:42px;border-bottom:1px solid var(--line);display:flex;padding:0 15px;gap:30px}.detail-tabs button{border:0;background:transparent;position:relative;font-weight:700}.detail-tabs .active{color:var(--blue)}.detail-tabs .active:after{content:"";position:absolute;bottom:0;left:0;right:0;height:3px;background:var(--blue)}.version-list,.link-list{padding:8px 15px;display:flex;flex-direction:column;gap:8px;overflow:auto}.version-list article{min-height:64px;border:1px solid #dbe4f0;border-radius:7px;display:flex;align-items:center;gap:10px;padding:6px;cursor:pointer}.version-list article.selected{border-color:var(--blue);background:#f3f7ff}.version-thumb{width:82px;height:48px;border-radius:5px;flex:0 0 auto}.version-list small{margin-left:8px;color:#6c80a1}.version-list p{font-size:11px;color:#637797;margin-top:4px}.version-list em{margin-left:auto;font-style:normal;color:var(--blue);font-size:11px}.version-list article>button{margin-left:auto;border:0;background:transparent;color:var(--blue);font-size:11px}.link-list article{height:58px;border:1px solid #dbe4f0;border-radius:7px;display:flex;align-items:center;gap:10px;padding:7px}.link-badge{width:37px;height:37px;display:grid;place-items:center;border-radius:7px;background:#eaf3ff;color:var(--blue);font-weight:700}.link-list article div{display:flex;flex-direction:column;gap:3px}.link-list small{font-size:11px;color:#697d9c}.link-list article button{margin-left:auto}.empty-links,.detail-empty{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:7px;color:#7184a3}.empty-links span{font-size:12px}.asset-detail>footer{margin-top:auto;border-top:1px solid var(--line);padding:13px 15px;display:grid;grid-template-columns:1fr 1fr;gap:10px}@media(max-width:1380px){.asset-layout{grid-template-columns:minmax(600px,1fr) 380px}.style-profile{height:145px}.style-items>div{height:88px}.mini-img,.ban{height:31px}.asset-grid{gap:8px}.detail-preview{height:155px}.head-actions{gap:6px}.status-pill{display:none}}
</style>
