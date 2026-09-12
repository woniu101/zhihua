<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { AlertTriangle, Check, Clock3, FileVideo2, FolderOpen, Maximize2, Monitor, Music2, Pause, Play, RotateCw, Subtitles, Upload, Volume2, VolumeX, X } from "lucide-vue-next";
import {
  chooseExportDirectory,
  defaultExportSettings,
  estimateOutputSizeMb,
  inspectExportCapability,
  inspectIntegrity,
  requestNativeExport,
  requestNativePreview,
  exportResolution,
  type ExportCapability,
  type ExportStatus,
} from "../services/exportService";
import { generationRepository, type CandidateAudioInspection, type CandidateVersion } from "../services/generationRepository";
import { ttsRepository } from "../services/ttsRepository";
import { assetRepository } from "../services/assetRepository";
import type { AssetItem } from "../domain/assets";
import { activeProjectId } from "../services/storyboardRepository";
import { useStoryboardStore } from "../stores/storyboard";

interface MixTrack { name: string; icon: string; volume: number; fade: boolean; muted: boolean }
type ExportPreset = "explainer" | "short" | "classroom" | "clean";

const settings = reactive(defaultExportSettings());
const exportPreset = ref<ExportPreset>("explainer");
const storyboard = useStoryboardStore();
const scenes = storyboard.scenes;
const mixTracks = reactive<MixTrack[]>([
  { name: "旁白", icon: "♩", volume: 80, fade: true, muted: false },
  { name: "音乐", icon: "♫", volume: 60, fade: true, muted: false },
  { name: "环境音", icon: "≋", volume: 28, fade: false, muted: false },
]);
const capability = ref<ExportCapability>({ available: false, label: "正在检测 FFmpeg", reason: "正在读取桌面导出能力。" });
const exportStatus = ref<ExportStatus>("checking");
const exportMessage = ref("正在执行完整性检查和 FFmpeg 能力检测。");
const lastCheckedAt = ref("");
const playhead = ref(0);
const fullPreviewUrl = ref("");
const previewBusy = ref(false);
const previewMessage = ref("尚未合成全片预览");
const selectedShotId = ref("");
const readyShotIds = ref<string[]>([]);
const enhancedShotIds = ref<string[]>([]);
const narrationShotIds = ref<string[]>([]);
const narrationIssueCount = ref(0);
const lastExportPath = ref("");
const audioAssets = ref<AssetItem[]>([]);
const selectedCandidates = ref<Record<string, CandidateVersion>>({});
const audioInspections = ref<Record<string, CandidateAudioInspection>>({});
const totalDuration = computed(() => scenes.value.reduce((sum, scene) => sum + scene.targetDurationMs / 1000, 0));
function formatTime(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds - minutes * 60;
  return `${String(minutes).padStart(2, "0")}:${remainder.toFixed(1).padStart(4, "0")}`;
}
const totalDurationText = computed(() => formatTime(totalDuration.value));
const selectedShot = computed(() => scenes.value.find((scene) => scene.id === selectedShotId.value) ?? scenes.value[0]);
const selectedCandidate = computed(() => selectedShot.value ? selectedCandidates.value[selectedShot.value.id] : undefined);
const selectedAudioInspection = computed(() => selectedCandidate.value ? audioInspections.value[selectedCandidate.value.id] : undefined);
const narrationTrack = computed(() => mixTracks.find((track) => track.name === "旁白"));
const narrationEnabled = computed(() => !narrationTrack.value?.muted && (narrationTrack.value?.volume ?? 0) > 0);

const integrityItems = computed(() => inspectIntegrity({
  totalShots: scenes.value.length,
  readyShotIds: readyShotIds.value,
  narrationComplete: !narrationEnabled.value || (scenes.value.length > 0 && narrationShotIds.value.length === scenes.value.length),
  narrationIssueCount: narrationEnabled.value ? narrationIssueCount.value : 0,
  subtitleComplete: settings.subtitleMode === "none" || (scenes.value.length > 0 && scenes.value.every((shot) => shot.narrationMode === "none" || Boolean(shot.narration.trim()))),
  sourceRecordsComplete: scenes.value.length > 0 && scenes.value.every((shot) => shot.sourceRefs.length > 0),
  missingAssetNames: [],
}));
const integrityPassed = computed(() => integrityItems.value.every((item) => item.passed));
const readyCount = computed(() => integrityItems.value.find((item) => item.id === "shots")?.detail ?? "0/0 就绪");
const estimatedSize = computed(() => estimateOutputSizeMb(totalDuration.value, settings.frameRate));
const resolutionText = computed(() => exportResolution(settings));
const playheadText = computed(() => formatTime(playhead.value));
const statusTone = computed(() => exportStatus.value === "succeeded" ? "success" : exportStatus.value === "exporting" || exportStatus.value === "checking" ? "working" : "blocked");

function savePreferences() {
  localStorage.setItem("zhihua.export.settings", JSON.stringify(settings));
  localStorage.setItem("zhihua.export.mix", JSON.stringify(mixTracks));
  localStorage.setItem("zhihua.export.preset", exportPreset.value);
}
function restorePreferences() {
  try {
    const storedSettings = JSON.parse(localStorage.getItem("zhihua.export.settings") ?? "null");
    if (storedSettings && typeof storedSettings === "object") Object.assign(settings, storedSettings);
    const storedMix = JSON.parse(localStorage.getItem("zhihua.export.mix") ?? "null");
    if (Array.isArray(storedMix)) mixTracks.splice(0, mixTracks.length, ...storedMix);
    const storedPreset = localStorage.getItem("zhihua.export.preset");
    if (["explainer", "short", "classroom", "clean"].includes(storedPreset ?? "")) exportPreset.value = storedPreset as ExportPreset;
  } catch {
    localStorage.removeItem("zhihua.export.settings");
    localStorage.removeItem("zhihua.export.mix");
  }
}
function setTrack(name: string, volume: number, muted: boolean) {
  const track = mixTracks.find((item) => item.name === name);
  if (track) Object.assign(track, { volume, muted });
}
function applyExportPreset() {
  settings.musicAssetId = "";
  if (exportPreset.value === "short") {
    settings.frameRate = 30;
    settings.subtitleMode = "burn-and-srt";
    settings.environmentAudioPolicy = "smart";
    setTrack("旁白", 88, false); setTrack("音乐", 55, true); setTrack("环境音", 32, false);
  } else if (exportPreset.value === "classroom") {
    settings.frameRate = 25;
    settings.subtitleMode = "burn-and-srt";
    settings.environmentAudioPolicy = "smart";
    setTrack("旁白", 90, false); setTrack("音乐", 45, true); setTrack("环境音", 18, false);
  } else if (exportPreset.value === "clean") {
    settings.frameRate = 24;
    settings.subtitleMode = "none";
    settings.environmentAudioPolicy = "off";
    setTrack("旁白", 80, true); setTrack("音乐", 60, true); setTrack("环境音", 28, true);
  } else {
    settings.frameRate = 24;
    settings.subtitleMode = "burn-and-srt";
    settings.environmentAudioPolicy = "smart";
    setTrack("旁白", 82, false); setTrack("音乐", 60, true); setTrack("环境音", 28, false);
  }
}
function resetMix() {
  mixTracks.splice(0, mixTracks.length,
    { name: "旁白", icon: "♩", volume: 80, fade: true, muted: false },
    { name: "音乐", icon: "♫", volume: 60, fade: true, muted: false },
    { name: "环境音", icon: "≋", volume: 28, fade: false, muted: false },
  );
}
function resetSettings() {
  Object.assign(settings, defaultExportSettings());
  settings.ratio = storyboard.settings.value.aspectRatio === "auto" ? "16:9" : storyboard.settings.value.aspectRatio;
  exportPreset.value = "explainer";
  applyExportPreset();
}
async function runChecks() {
  exportStatus.value = "checking";
  exportMessage.value = "正在重新检查分镜、素材和桌面导出能力。";
  await refreshProjectArtifacts();
  capability.value = await inspectExportCapability();
  lastCheckedAt.value = new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }).format(new Date());
  if (!integrityPassed.value) {
    exportStatus.value = "blocked";
    exportMessage.value = `当前仅 ${readyCount.value}，请先在分镜页为全部镜头选择正式版本。`;
  } else if (!capability.value.available) {
    exportStatus.value = "blocked";
    exportMessage.value = capability.value.reason;
  } else {
    exportStatus.value = "idle";
    exportMessage.value = "检查通过，可以导出 MP4。";
  }
}
async function beginExport() {
  await runChecks();
  if (!integrityPassed.value || !capability.value.available) return;
  exportStatus.value = "exporting";
  exportMessage.value = "正在调用 FFmpeg 合成视频。";
  try {
    const projectId = activeProjectId();
    if (!projectId) throw new Error("请先打开一个项目");
    const narration = mixTracks.find((track) => track.name === "旁白");
    const music = mixTracks.find((track) => track.name === "音乐");
    const environment = mixTracks.find((track) => track.name === "环境音");
    const result = await requestNativeExport(
      projectId,
      { ...settings },
      narration?.muted ? 0 : narration?.volume ?? 80,
      music?.muted ? 0 : music?.volume ?? 60,
      music?.fade ?? true,
      environment?.muted ? 0 : environment?.volume ?? 28,
    );
    lastExportPath.value = result.outputPath;
    exportStatus.value = "succeeded";
    const sourceSummary = settings.rendition === "enhanced-1080p"
      ? `增强源 ${result.enhancedSceneCount} 个，候选放大 ${result.scaledSceneCount} 个`
      : "使用正式候选版本";
    exportMessage.value = `导出完成（${result.width}×${result.height}，${sourceSummary}）：${result.outputPath}`;
  } catch (error) {
    exportStatus.value = "failed";
    exportMessage.value = error instanceof Error ? error.message : "导出失败。";
  }
}

async function refreshProjectArtifacts() {
  await storyboard.load(true);
  selectedShotId.value ||= scenes.value[0]?.id ?? "";
  const projectId = activeProjectId();
  audioAssets.value = projectId
    ? (await assetRepository.list(projectId).catch(() => [])).filter((asset) => asset.mediaType === "audio")
    : [];
  const candidateMap: Record<string, CandidateVersion> = {};
  const states = await Promise.all(scenes.value.map(async (scene) => {
    const [candidates, enhancedVersions, narration] = await Promise.all([
      generationRepository.list(scene.projectId, scene.id).catch(() => []),
      generationRepository.listEnhanced(scene.projectId, scene.id).catch(() => []),
      ttsRepository.get(scene.projectId, scene.id).catch(() => undefined),
    ]);
    const narrationHash = narration
      ? Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(scene.narration))))
        .map((byte) => byte.toString(16).padStart(2, "0"))
        .join("")
      : "";
    const narrationReady = scene.narrationMode === "none" || Boolean(
      narration
      && narration.textSha256 === narrationHash
      && narration.durationMs <= scene.targetDurationMs + 250,
    );
    const selected = candidates.find((item) => item.id === scene.selectedVersionId);
    if (selected) candidateMap[scene.id] = selected;
    return {
      id: scene.id,
      candidateReady: Boolean(scene.selectedVersionId && candidates.some((item) => (
        item.id === scene.selectedVersionId && item.aspectRatio === settings.ratio
      ))),
      enhancedReady: Boolean(scene.selectedVersionId && enhancedVersions.some((item) => item.sourceCandidateId === scene.selectedVersionId)),
      narrationReady,
      narrationIssue: scene.narrationMode !== "none" && Boolean(narration) && !narrationReady,
    };
  }));
  selectedCandidates.value = candidateMap;
  const inspections = await Promise.allSettled(Object.values(candidateMap).map((candidate) => generationRepository.inspectAudio(candidate.id)));
  audioInspections.value = Object.fromEntries(inspections
    .filter((result): result is PromiseFulfilledResult<CandidateAudioInspection> => result.status === "fulfilled")
    .map((result) => [result.value.candidateId, result.value]));
  readyShotIds.value = states.filter((item) => item.candidateReady).map((item) => item.id);
  enhancedShotIds.value = states.filter((item) => item.enhancedReady).map((item) => item.id);
  narrationShotIds.value = states.filter((item) => item.narrationReady).map((item) => item.id);
  narrationIssueCount.value = states.filter((item) => item.narrationIssue).length;
}

async function browseOutputDirectory() {
  const selected = await chooseExportDirectory();
  if (selected) settings.outputDirectory = selected;
}

async function buildFullPreview() {
  const projectId = activeProjectId();
  if (!projectId || previewBusy.value) return;
  await runChecks();
  if (!integrityPassed.value || !capability.value.available) {
    previewMessage.value = exportMessage.value;
    return;
  }
  previewBusy.value = true;
  previewMessage.value = "正在用正式版本、旁白和字幕合成预览。";
  try {
    const narration = mixTracks.find((track) => track.name === "旁白");
    const music = mixTracks.find((track) => track.name === "音乐");
    const environment = mixTracks.find((track) => track.name === "环境音");
    const result = await requestNativePreview(
      projectId,
      { ...settings },
      narration?.muted ? 0 : narration?.volume ?? 80,
      music?.muted ? 0 : music?.volume ?? 60,
      music?.fade ?? true,
      environment?.muted ? 0 : environment?.volume ?? 28,
    );
    fullPreviewUrl.value = convertFileSrc(result.outputPath);
    playhead.value = 0;
    previewMessage.value = `全片预览已更新 · ${formatTime(result.durationMs / 1000)} · ${result.width}×${result.height}`;
  } catch (error) {
    previewMessage.value = error instanceof Error ? error.message : String(error);
  } finally {
    previewBusy.value = false;
  }
}

function updatePlayhead(event: Event) {
  playhead.value = (event.target as HTMLVideoElement).currentTime;
}

watch(settings, savePreferences, { deep: true });
watch(mixTracks, savePreferences, { deep: true });
watch(exportPreset, savePreferences);
onMounted(async () => {
  restorePreferences();
  await storyboard.load(true);
  settings.ratio = storyboard.settings.value.aspectRatio === "auto" ? "16:9" : storyboard.settings.value.aspectRatio;
  settings.environmentAudioPolicy = storyboard.settings.value.h3AudioPolicy;
  selectedShotId.value = scenes.value[0]?.id ?? "";
  playhead.value = Math.min(playhead.value, totalDuration.value);
  await runChecks();
});
</script>

<template>
  <section class="page export-page">
    <header class="export-head">
      <div class="page-title-line"><h1>检查并导出</h1><span class="ready" :class="statusTone"><span class="status-symbol"><Check v-if="integrityPassed && capability.available" :size="14"/><AlertTriangle v-else :size="15"/></span><b>{{ exportStatus==='checking' ? '正在检查' : exportStatus==='blocked' ? '暂不可导出' : exportStatus==='exporting' ? '正在导出' : exportStatus==='failed' ? '导出失败' : exportStatus==='succeeded' ? '导出完成' : '准备就绪' }}</b><small>{{ exportMessage }}</small></span></div>
      <button class="btn" @click="runChecks"><RotateCw :size="17"/>重新检查</button>
    </header>

    <div class="export-layout">
      <section class="export-left">
        <div class="panel full-preview">
          <div class="preview-toolbar"><div><b>{{ fullPreviewUrl ? '全片预览' : selectedCandidate ? `镜头预览 · ${selectedShot?.title}` : '成片预览' }}</b><span>{{ fullPreviewUrl ? previewMessage : '先逐镜头核对正式版本，再生成一次完整成片预览' }}</span><span v-if="!fullPreviewUrl && selectedAudioInspection" class="audio-inspection" :class="{blocked:selectedAudioInspection.speechDetected}">{{ selectedAudioInspection.detail }}</span></div><button class="btn primary" :disabled="previewBusy" @click="buildFullPreview"><RotateCw :size="16"/>{{ previewBusy ? '正在合成' : fullPreviewUrl ? '更新全片预览' : '生成全片预览' }}</button></div>
          <div class="export-video"><video v-if="fullPreviewUrl" :src="fullPreviewUrl" controls @timeupdate="updatePlayhead"></video><video v-else-if="selectedCandidate" :key="selectedCandidate.id" :src="selectedCandidate.previewUrl" controls @timeupdate="updatePlayhead"></video><div v-else class="empty-video"><FileVideo2 :size="42"/><b>还没有可预览的正式版本</b><span>前往分镜页生成候选，并为每个镜头选定正式版本。</span></div><span>{{ fullPreviewUrl ? resolutionText : selectedCandidate ? `${selectedCandidate.visibleWidth}×${selectedCandidate.visibleHeight}` : resolutionText }}　{{ settings.ratio }}</span><div v-if="!fullPreviewUrl && selectedCandidate" class="caption">{{ selectedShot?.narration }}</div></div>
        </div>

        <section class="panel review-panel">
          <div class="panel-head"><div><h2>镜头与节奏</h2><span>点击镜头预览正式版本</span></div><b>{{ readyCount }} · {{ totalDurationText }}</b></div>
          <div class="shot-strip"><article v-for="(shot,index) in scenes" :key="shot.id" :class="{active:selectedShotId===shot.id}" @click="selectedShotId=shot.id"><div><video v-if="selectedCandidates[shot.id]" :src="selectedCandidates[shot.id].previewUrl" muted preload="metadata"></video><span v-else class="missing-thumb">待选择</span><b>{{ String(index+1).padStart(2,'0') }}</b><time>{{ shot.targetDurationMs / 1000 }}秒</time></div><span>{{ shot.title }}</span></article><p v-if="!scenes.length" class="empty-export">当前项目还没有分镜</p></div>
          <div class="final-timeline"><div class="timeline-meta"><span>00:00</span><span>播放位置 {{ playheadText }}</span><span>{{ totalDurationText }}</span></div><div class="final-track"><i v-for="(shot,index) in scenes" :key="shot.id" :class="{notReady:!readyShotIds.includes(shot.id)}">{{ String(index+1).padStart(2,'0') }}<X v-if="!readyShotIds.includes(shot.id)" :size="12"/></i><span class="marker" :style="{left:`${totalDuration ? playhead/totalDuration*100 : 0}%`}"></span></div></div>
        </section>
      </section>

      <aside class="export-right">
        <section class="panel readiness-panel">
          <div class="panel-head"><h2><span class="check-big" :class="{failed:!integrityPassed}"><Check v-if="integrityPassed" :size="17"/><AlertTriangle v-else :size="16"/></span>导出准备</h2><b :class="integrityPassed?'success-text':'warning-text'">{{ integrityPassed ? '全部通过' : '需要处理' }}</b></div>
          <div class="check-grid"><p v-for="item in integrityItems" :key="item.id" :class="{failed:!item.passed}"><span class="check"><Check v-if="item.passed" :size="13"/><X v-else :size="13"/></span><span><b>{{ item.label }}</b><small>{{ item.detail }}</small></span></p></div>
          <div class="export-facts"><article><Monitor :size="20"/><span>清晰度<b>{{ resolutionText }}</b></span></article><article><Clock3 :size="20"/><span>总时长<b>{{ totalDuration }} 秒</b></span></article><article><FileVideo2 :size="20"/><span>预计大小<b>约 {{ estimatedSize }} MB</b></span></article><em class="capability-badge" :class="{available:capability.available}">{{ capability.label }}</em></div>
        </section>

        <section class="panel export-settings">
          <div class="panel-head"><div><h2>成片设置</h2><span>按用途调整，预览和正式导出使用同一套参数</span></div><button class="btn" @click="resetSettings">恢复默认</button></div>
          <div class="settings-grid">
            <label><span><FileVideo2 :size="15"/>成片用途</span><select v-model="exportPreset" @change="applyExportPreset"><option value="explainer">讲解成片（推荐）</option><option value="short">短视频发布</option><option value="classroom">课堂演示</option><option value="clean">纯画面素材</option></select></label>
            <label><span><Monitor :size="15"/>输出清晰度</span><select v-model="settings.rendition"><option value="candidate">候选原清晰度（快速）</option><option value="enhanced-1080p">1080p（优先增强版）</option></select></label>
            <label><span><FileVideo2 :size="15"/>格式</span><select v-model="settings.videoCodec"><option value="H.264">MP4 · H.264（通用）</option></select></label>
            <label><span><Monitor :size="15"/>帧率</span><select v-model.number="settings.frameRate"><option :value="24">24 fps（电影感）</option><option :value="25">25 fps</option><option :value="30">30 fps（更流畅）</option></select></label>
            <label><span><Volume2 :size="15"/>H3 环境音</span><select v-model="settings.environmentAudioPolicy"><option value="smart">智能使用（推荐）</option><option value="always">始终使用</option><option value="off">不使用</option></select></label>
            <label><span><Subtitles :size="15"/>字幕</span><select v-model="settings.subtitleMode"><option value="burn-and-srt">嵌入画面并保存 SRT</option><option value="burn">仅嵌入画面</option><option value="srt">仅保存 SRT</option><option value="none">关闭字幕</option></select></label>
            <label><span><Music2 :size="15"/>背景音乐</span><select v-model="settings.musicAssetId"><option value="">不添加背景音乐</option><option v-for="asset in audioAssets" :key="asset.id" :value="asset.id">{{ asset.name }}</option></select></label>
            <label><span><Monitor :size="15"/>项目画幅</span><select :value="settings.ratio" disabled title="项目画幅请在分镜页修改"><option :value="settings.ratio">{{ settings.ratio }}（跟随项目）</option></select></label>
            <label class="path-row"><span><FolderOpen :size="15"/>保存位置</span><div><input v-model.trim="settings.outputDirectory" placeholder="项目 exports 目录" aria-label="导出保存位置"/><button type="button" @click="browseOutputDirectory">浏览</button></div></label>
          </div>

          <div class="mix-section"><div class="mix-title"><div><h3>声音</h3><span>旁白优先，环境音会在旁白出现时自动降低</span></div><button @click="resetMix">恢复默认</button></div><div class="mix-row" v-for="track in mixTracks" :key="track.name"><span class="mix-icon">{{ track.icon }}</span><b>{{ track.name }}</b><input v-model.number="track.volume" :aria-label="`${track.name}音量`" type="range" min="0" max="100" :disabled="track.muted"/><span>{{ track.muted ? '静音' : `${track.volume}%` }}</span><button class="mute-button" :aria-label="`${track.name}${track.muted?'取消静音':'静音'}`" @click="track.muted=!track.muted"><VolumeX v-if="track.muted" :size="17"/><Volume2 v-else :size="17"/></button><label><input v-model="track.fade" type="checkbox" :disabled="track.name==='环境音'"/>{{ track.name==='环境音' ? '旁白时降低' : '淡入淡出' }}</label></div></div>

          <div class="export-action"><div class="export-state" :class="statusTone"><b>{{ capability.available ? (integrityPassed ? '已准备好导出' : '完成左侧阻塞项后即可导出') : 'FFmpeg 不可用' }}</b><span>{{ settings.rendition === 'enhanced-1080p' ? `AI 增强版 ${enhancedShotIds.length}/${scenes.length}；其余镜头将高质量放大。` : (lastExportPath ? `最近导出：${lastExportPath}` : exportMessage) }}</span></div><button class="btn primary export-btn" :disabled="exportStatus==='checking'||exportStatus==='exporting'||!integrityPassed||!capability.available" @click="beginExport"><Upload :size="20"/>{{ exportStatus==='exporting' ? '正在导出' : '导出 MP4' }}</button></div>
        </section>
      </aside>
    </div>
  </section>
</template>

<style scoped>
.export-page{display:grid;grid-template-rows:64px minmax(0,1fr);gap:10px}.export-head{display:flex;align-items:center;justify-content:space-between;padding:0 6px}.ready{display:grid;grid-template-columns:24px auto;gap:1px 9px;align-items:center}.ready .status-symbol{grid-row:1/3;width:22px;height:22px;border-radius:50%;display:grid;place-items:center;color:white;background:var(--orange)}.ready.success .status-symbol{background:var(--green)}.ready.working .status-symbol{background:var(--blue)}.ready b{font-size:15px}.ready small{color:#697d9f;max-width:590px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.export-layout{min-height:0;display:grid;grid-template-columns:minmax(620px,1.58fr) minmax(450px,1fr);gap:14px}.export-left{min-height:0;display:grid;grid-template-rows:minmax(360px,1fr) 252px;gap:12px}.full-preview{overflow:hidden;display:flex;flex-direction:column}.preview-toolbar{min-height:58px;padding:8px 14px;border-bottom:1px solid var(--line);display:flex;align-items:center;justify-content:space-between;gap:12px}.preview-toolbar>div{min-width:0;display:flex;flex-direction:column;gap:3px}.preview-toolbar b{font-size:16px}.preview-toolbar span,.panel-head>div>span{color:var(--muted);font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.preview-toolbar .audio-inspection{color:#087d51}.preview-toolbar .audio-inspection.blocked{color:#b34b19}.preview-toolbar .btn{min-height:38px;white-space:nowrap}.export-video{flex:1;position:relative;min-height:0;background:#07101f}.export-video video{width:100%;height:100%;display:block;object-fit:contain}.export-video>span{position:absolute;top:12px;left:13px;color:#fff;background:rgba(4,15,31,.79);padding:6px 10px;border-radius:5px;font-size:13px}.caption{position:absolute;left:50%;transform:translateX(-50%);bottom:16px;color:white;background:rgba(4,15,31,.76);padding:8px 15px;border-radius:5px;font-size:18px;white-space:nowrap}.empty-video{position:absolute;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:9px;color:#8ea0bc;background:#0b1728}.empty-video b{color:#dfe8f5;font-size:18px}.empty-video span{font-size:13px}.review-panel{overflow:hidden}.review-panel .panel-head{height:48px}.review-panel .panel-head>div{min-width:0}.review-panel .panel-head h2{font-size:17px}.review-panel .panel-head>b{color:#526b92;font-size:13px}.shot-strip{height:133px;display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:8px;padding:8px 12px 5px}.shot-strip article{min-width:0;padding:3px;border-radius:7px;cursor:pointer}.shot-strip article.active{background:#edf5ff}.shot-strip article>div{height:82px;border-radius:6px;position:relative;border:2px solid transparent;overflow:hidden}.shot-strip article.active>div{border-color:var(--blue)}.shot-strip video{width:100%;height:100%;display:block;object-fit:cover;border-radius:4px}.shot-strip article>div>b,.shot-strip time{position:absolute;z-index:2;color:#fff;background:#06182f;padding:3px 5px;border-radius:4px;font-size:11px}.shot-strip article>div>b{top:4px;left:4px}.shot-strip time{right:4px;bottom:4px}.shot-strip article>span{display:block;margin:4px 2px 0;font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.missing-thumb{position:absolute;inset:0;display:grid;place-items:center;color:#7184a3;background:#edf3fa}.empty-export{grid-column:1/-1;display:grid;place-items:center;color:var(--muted)}.final-timeline{height:70px;padding:0 14px}.timeline-meta{height:22px;display:flex;align-items:center;justify-content:space-between;color:#637797;font-size:11px}.final-track{height:34px;display:flex;position:relative;border-radius:5px;overflow:visible}.final-track i{flex:1;display:flex;align-items:center;justify-content:center;gap:3px;font-style:normal;background:#cfe2ff;border-right:2px solid white;font-size:11px}.final-track i:first-child{border-radius:5px 0 0 5px}.final-track i:last-of-type{border-radius:0 5px 5px 0;border:0}.final-track i:nth-child(2n){background:#d8f2e8}.final-track i.notReady{color:#a75416;background:#ffebc6}.marker{position:absolute;top:-4px;bottom:-5px;width:2px;background:#ff394e}.marker:before{content:"";position:absolute;top:-1px;left:-4px;border-left:5px solid transparent;border-right:5px solid transparent;border-top:8px solid #ff394e}
.export-right{min-height:0;display:grid;grid-template-rows:254px minmax(0,1fr);gap:12px}.readiness-panel{overflow:hidden}.readiness-panel .panel-head h2{display:flex;align-items:center;gap:9px}.check-big,.check{border-radius:50%;display:grid;place-items:center;color:#fff;background:var(--green)}.check-big{width:25px;height:25px}.check-big.failed,.check-grid p.failed .check{background:#f0644c}.check-grid{display:grid;grid-template-columns:1fr 1fr;gap:7px;padding:10px 14px}.check-grid p{min-width:0;height:48px;padding:7px 9px;border:1px solid #e0e8f3;border-radius:7px;display:grid;grid-template-columns:22px minmax(0,1fr);align-items:center;gap:6px;background:#f9fbfe}.check-grid p.failed{border-color:#ffd6c9;background:#fff8f4}.check{width:20px;height:20px}.check-grid p>span:last-child{min-width:0;display:flex;flex-direction:column}.check-grid p b{font-size:12px}.check-grid p small{color:#6b7f9f;font-size:10px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.export-facts{height:67px;margin:0 14px;padding:8px 10px;border-top:1px solid #e5ebf4;display:grid;grid-template-columns:1fr 1fr 1fr auto;align-items:center;gap:8px}.export-facts article{display:flex;align-items:center;gap:7px;min-width:0;color:var(--blue)}.export-facts article span{display:flex;flex-direction:column;color:#667b9c;font-size:10px}.export-facts article b{color:var(--text);font-size:13px;white-space:nowrap}.capability-badge{font-style:normal;font-size:10px;color:#c34f15;background:#fff0e7;padding:5px 7px;border-radius:5px;white-space:nowrap}.capability-badge.available{color:#078c57;background:#e8f8f1}
.export-settings{min-height:0;display:flex;flex-direction:column;padding-bottom:12px;overflow:hidden}.export-settings .panel-head{height:52px}.export-settings .panel-head>div{min-width:0}.export-settings .panel-head h2{font-size:17px}.export-settings .panel-head .btn{min-height:32px}.settings-grid{display:grid;grid-template-columns:1fr 1fr;gap:8px 10px;padding:10px 14px}.settings-grid label{display:flex;flex-direction:column;gap:4px;min-width:0}.settings-grid label>span{height:17px;display:flex;align-items:center;gap:5px;color:#405879;font-size:11px}.settings-grid select,.settings-grid input{height:32px;min-width:0;padding:0 9px;border:1px solid #d2deee;border-radius:6px;background:#fff;font-size:12px;color:var(--text)}.settings-grid .path-row{grid-column:1/-1}.path-row>div{display:grid;grid-template-columns:minmax(0,1fr) 54px;gap:5px}.path-row button{height:32px;border:1px solid #d2deee;border-radius:6px;background:#f3f6fa;color:#526b92}.mix-section{margin:0 14px;padding:8px 10px;border-radius:8px;background:#f6f9fd}.mix-title{height:30px;display:flex;justify-content:space-between;align-items:flex-start}.mix-title h3{font-size:13px}.mix-title span{display:block;color:#6b7f9f;font-size:10px}.mix-title button{border:0;background:transparent;color:var(--blue);font-size:11px}.mix-row{height:30px;display:grid;grid-template-columns:20px 43px minmax(65px,1fr) 37px 24px 76px;align-items:center;gap:5px}.mix-icon{font-size:18px;color:#274b80}.mix-row b,.mix-row span,.mix-row label{font-size:11px}.mix-row input[type=range]{min-width:0;accent-color:var(--blue)}.mix-row label{white-space:nowrap}.mute-button{border:0;background:transparent;display:grid;place-items:center;color:#37537d}.export-action{margin-top:auto;padding:10px 14px 0;display:grid;grid-template-columns:minmax(0,1fr) 150px;gap:10px;align-items:stretch;border-top:1px solid #e3eaf4}.export-state{min-width:0;padding:8px 10px;border-radius:7px;background:#fff1e9;color:#8f3f18;display:flex;flex-direction:column;justify-content:center;gap:2px}.export-state.success{background:#e8f8f1;color:#087d51}.export-state.working{background:#eaf3ff;color:#0a5fd5}.export-state span{font-size:10px;line-height:1.3;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.export-btn{min-height:48px;font-size:16px}.export-btn:disabled{opacity:.65;cursor:wait}
@media(max-width:1380px){.export-left{grid-template-rows:minmax(320px,1fr) 226px}.review-panel .panel-head{height:44px}.shot-strip{height:118px}.shot-strip article>div{height:68px}.export-right{grid-template-rows:224px minmax(0,1fr)}.check-grid{gap:5px;padding:7px 10px}.check-grid p{height:41px;padding:5px 7px}.export-facts{height:56px;margin:0 10px;padding:5px}.settings-grid{gap:5px 8px;padding:7px 10px}.settings-grid select,.settings-grid input{height:29px}.mix-section{margin:0 10px;padding:5px 8px}.mix-row{height:27px}.export-action{padding:7px 10px 0}.export-btn{min-height:42px}}
</style>
