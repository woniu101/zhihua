<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { AlertTriangle, Check, Clock3, FileVideo2, FolderOpen, Maximize2, Monitor, Music2, Pause, Play, RotateCw, Subtitles, Upload, Volume2, VolumeX, X } from "lucide-vue-next";
import {
  chooseExportDirectory,
  defaultExportSettings,
  estimateOutputSizeMb,
  inspectExportCapability,
  inspectIntegrity,
  requestNativeExport,
  exportResolution,
  type ExportCapability,
  type ExportStatus,
} from "../services/exportService";
import { generationRepository, type CandidateVersion } from "../services/generationRepository";
import { ttsRepository } from "../services/ttsRepository";
import { assetRepository } from "../services/assetRepository";
import type { AssetItem } from "../domain/assets";
import { activeProjectId } from "../services/storyboardRepository";
import { useStoryboardStore } from "../stores/storyboard";

interface MixTrack { name: string; icon: string; volume: number; fade: boolean; muted: boolean }

const settings = reactive(defaultExportSettings());
const storyboard = useStoryboardStore();
const scenes = storyboard.scenes;
const mixTracks = reactive<MixTrack[]>([
  { name: "旁白", icon: "♩", volume: 80, fade: true, muted: false },
  { name: "音乐", icon: "♫", volume: 60, fade: true, muted: false },
  { name: "环境音", icon: "≋", volume: 50, fade: false, muted: true },
]);
const capability = ref<ExportCapability>({ available: false, label: "正在检测 FFmpeg", reason: "正在读取桌面导出能力。" });
const exportStatus = ref<ExportStatus>("checking");
const exportMessage = ref("正在执行完整性检查和 FFmpeg 能力检测。");
const lastCheckedAt = ref("");
const playing = ref(false);
const playhead = ref(18.4);
const previewVolume = ref(62);
const selectedShotId = ref("");
const readyShotIds = ref<string[]>([]);
const enhancedShotIds = ref<string[]>([]);
const narrationShotIds = ref<string[]>([]);
const narrationIssueCount = ref(0);
const lastExportPath = ref("");
const audioAssets = ref<AssetItem[]>([]);
const selectedCandidates = ref<Record<string, CandidateVersion>>({});
const totalDuration = computed(() => scenes.value.reduce((sum, scene) => sum + scene.targetDurationMs / 1000, 0));
function formatTime(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds - minutes * 60;
  return `${String(minutes).padStart(2, "0")}:${remainder.toFixed(1).padStart(4, "0")}`;
}
const totalDurationText = computed(() => formatTime(totalDuration.value));
const selectedShot = computed(() => scenes.value.find((scene) => scene.id === selectedShotId.value) ?? scenes.value[0]);
const selectedCandidate = computed(() => selectedShot.value ? selectedCandidates.value[selectedShot.value.id] : undefined);

const integrityItems = computed(() => inspectIntegrity({
  totalShots: scenes.value.length,
  readyShotIds: readyShotIds.value,
  narrationComplete: scenes.value.length > 0 && narrationShotIds.value.length === scenes.value.length,
  narrationIssueCount: narrationIssueCount.value,
  subtitleComplete: scenes.value.length > 0 && scenes.value.every((shot) => Boolean(shot.narration.trim())),
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
}
function restorePreferences() {
  try {
    const storedSettings = JSON.parse(localStorage.getItem("zhihua.export.settings") ?? "null");
    if (storedSettings && typeof storedSettings === "object") Object.assign(settings, storedSettings);
    const storedMix = JSON.parse(localStorage.getItem("zhihua.export.mix") ?? "null");
    if (Array.isArray(storedMix)) mixTracks.splice(0, mixTracks.length, ...storedMix);
    const environment = mixTracks.find((track) => track.name === "环境音");
    if (environment) {
      environment.muted = true;
      environment.fade = false;
    }
  } catch {
    localStorage.removeItem("zhihua.export.settings");
    localStorage.removeItem("zhihua.export.mix");
  }
}
function resetMix() {
  mixTracks.splice(0, mixTracks.length,
    { name: "旁白", icon: "♩", volume: 80, fade: true, muted: false },
    { name: "音乐", icon: "♫", volume: 60, fade: true, muted: false },
    { name: "环境音", icon: "≋", volume: 50, fade: false, muted: true },
  );
}
function resetSettings() {
  Object.assign(settings, defaultExportSettings());
  settings.ratio = storyboard.settings.value.aspectRatio === "auto" ? "16:9" : storyboard.settings.value.aspectRatio;
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
    const narrationVolume = mixTracks.find((track) => track.name === "旁白")?.volume ?? 80;
    const music = mixTracks.find((track) => track.name === "音乐");
    const result = await requestNativeExport(
      projectId,
      { ...settings },
      narrationVolume,
      music?.muted ? 0 : music?.volume ?? 60,
      music?.fade ?? true,
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
    const narrationReady = Boolean(
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
      narrationIssue: Boolean(narration) && !narrationReady,
    };
  }));
  selectedCandidates.value = candidateMap;
  readyShotIds.value = states.filter((item) => item.candidateReady).map((item) => item.id);
  enhancedShotIds.value = states.filter((item) => item.enhancedReady).map((item) => item.id);
  narrationShotIds.value = states.filter((item) => item.narrationReady).map((item) => item.id);
  narrationIssueCount.value = states.filter((item) => item.narrationIssue).length;
}

async function browseOutputDirectory() {
  const selected = await chooseExportDirectory();
  if (selected) settings.outputDirectory = selected;
}

watch(settings, savePreferences, { deep: true });
watch(mixTracks, savePreferences, { deep: true });
onMounted(async () => {
  restorePreferences();
  await storyboard.load(true);
  settings.ratio = storyboard.settings.value.aspectRatio === "auto" ? "16:9" : storyboard.settings.value.aspectRatio;
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
        <div class="panel full-preview"><div class="export-video"><video v-if="selectedCandidate" :key="selectedCandidate.id" :src="selectedCandidate.previewUrl" controls></video><div v-else class="empty-video"><FileVideo2 :size="42"/><b>当前分镜没有正式候选</b><span>请先在分镜页选择一个候选版本作为正式版本。</span></div><span>{{ selectedCandidate ? `${selectedCandidate.visibleWidth}×${selectedCandidate.visibleHeight}` : resolutionText }}　{{ settings.ratio }}</span><div v-if="selectedCandidate" class="caption">{{ selectedShot?.narration }}</div></div></div>
        <div class="shot-strip"><article v-for="(shot,index) in scenes" :key="shot.id" :class="{active:selectedShotId===shot.id}" @click="selectedShotId=shot.id"><div><video v-if="selectedCandidates[shot.id]" :src="selectedCandidates[shot.id].previewUrl" muted preload="metadata"></video><span v-else class="missing-thumb">待选择</span><b>{{ String(index+1).padStart(2,'0') }}</b><time>{{ shot.targetDurationMs / 1000 }}秒</time></div><span>{{ shot.title }}</span></article><p v-if="!scenes.length" class="empty-export">当前项目还没有分镜</p></div>
        <div class="panel mix-panel"><div class="panel-head"><h2>音频混音</h2><button class="btn" @click="resetMix">恢复默认</button></div><div class="mix-row" v-for="track in mixTracks" :key="track.name"><span class="mix-icon">{{ track.icon }}</span><b>{{ track.name }}</b><input v-model.number="track.volume" :aria-label="`${track.name}音量`" class="native-range" type="range" min="0" max="100" :disabled="track.muted || track.name==='环境音'"/><span>{{ track.name==='环境音' ? '暂未启用' : track.muted ? '静音' : `${track.volume}%` }}</span><button class="mute-button" :disabled="track.name==='环境音'" :aria-label="`${track.name}${track.muted?'取消静音':'静音'}`" @click="track.muted=!track.muted"><VolumeX v-if="track.muted" :size="17"/><Volume2 v-else :size="17"/></button><label><input v-model="track.fade" type="checkbox" :disabled="track.name==='环境音'"/> 应用淡入淡出</label></div></div>
        <div class="panel final-timeline"><div class="panel-head"><h3>最终检查时间线</h3><span>{{ playheadText }}　　　　　　　　　总时长 {{ totalDurationText }}</span></div><div class="final-track"><i v-for="(shot,index) in scenes" :key="shot.id" :class="{notReady:!readyShotIds.includes(shot.id)}">{{ String(index+1).padStart(2,'0') }}<X v-if="!readyShotIds.includes(shot.id)" :size="12"/></i><span class="marker" :style="{left:`${totalDuration ? playhead/totalDuration*100 : 0}%`}"></span></div><div class="ticks"><span>0:00</span><span>25%</span><span>50%</span><span>75%</span><span>{{ totalDurationText }}</span></div></div>
      </section>

      <aside class="export-right">
        <section class="panel checklist"><div class="panel-head"><h2><span class="check-big" :class="{failed:!integrityPassed}"><Check v-if="integrityPassed" :size="17"/><AlertTriangle v-else :size="16"/></span>完整性检查</h2><b :class="integrityPassed?'success-text':'warning-text'">{{ integrityPassed ? '全部通过' : '存在阻塞项' }}</b></div><p v-for="item in integrityItems" :key="item.id" :class="{failed:!item.passed}"><span class="check"><Check v-if="item.passed" :size="15"/><X v-else :size="15"/></span><b>{{ item.label }}</b><span>{{ item.detail }}</span></p><small v-if="lastCheckedAt">最近检查 {{ lastCheckedAt }}</small></section>
        <section class="panel export-info"><div class="panel-head"><h2>导出信息</h2><b class="capability-badge" :class="{available:capability.available}">{{ capability.label }}</b></div><div><article><Monitor :size="31"/><span>分辨率</span><b>{{ resolutionText }}</b><small>{{ settings.ratio }}</small></article><article><Clock3 :size="31"/><span>总时长</span><b>{{ totalDuration }} 秒</b></article><article><FileVideo2 :size="31"/><span>预计文件大小</span><b>约 {{ estimatedSize }} MB</b></article></div></section>
        <section class="panel export-settings"><div class="panel-head"><h2>导出设置</h2><button class="btn" @click="resetSettings">恢复默认</button></div><div class="settings-list"><label><FileVideo2 :size="18"/><span>导出格式</span><select v-model="settings.videoCodec"><option value="H.264">MP4 · H.264（通用，推荐）</option></select></label><label><Monitor :size="18"/><span>输出清晰度</span><select v-model="settings.rendition"><option value="candidate">候选原清晰度（快速）</option><option value="enhanced-1080p">1080p（优先增强版，缺失时高质量放大）</option></select></label><label><Music2 :size="18"/><span>背景音乐</span><select v-model="settings.musicAssetId"><option value="">不添加背景音乐</option><option v-for="asset in audioAssets" :key="asset.id" :value="asset.id">{{ asset.name }}</option></select></label><label><Monitor :size="18"/><span>帧率</span><select v-model.number="settings.frameRate"><option :value="24">24 fps（电影感）</option><option :value="25">25 fps</option><option :value="30">30 fps（更流畅）</option></select></label><label><Monitor :size="18"/><span>项目画幅</span><select :value="settings.ratio" disabled title="项目画幅请在分镜页修改"><option :value="settings.ratio">{{ settings.ratio }}（跟随项目）</option></select></label><label><Subtitles :size="18"/><span>字幕处理</span><select v-model="settings.subtitleMode"><option value="burn-and-srt">嵌入画面并保存 SRT</option><option value="burn">仅嵌入画面</option><option value="srt">仅保存 SRT</option></select></label><label class="path-row"><FolderOpen :size="18"/><span>保存位置</span><input v-model.trim="settings.outputDirectory" placeholder="留空保存到项目 exports 目录" aria-label="导出保存位置"/><button type="button" @click="browseOutputDirectory">浏览</button></label></div><button class="btn primary export-btn" :disabled="exportStatus==='checking'||exportStatus==='exporting'" @click="beginExport"><Upload :size="20"/>{{ exportStatus==='exporting' ? '正在导出' : '导出 MP4' }}</button><div class="export-state" :class="statusTone"><b>{{ capability.available ? (integrityPassed ? '可执行导出' : '等待分镜就绪') : 'FFmpeg 不可用' }}</b><span>{{ settings.rendition === 'enhanced-1080p' ? `已有 ${enhancedShotIds.length}/${scenes.length} 个镜头具备 AI 增强版；其余将从正式候选高质量放大。` : (capability.available ? exportMessage : capability.reason) }}</span></div><small>{{ lastExportPath ? `最近导出：${lastExportPath}` : '正式内容版本与输出清晰度相互独立，可随时重新导出。' }}</small></section>
      </aside>
    </div>
  </section>
</template>

<style scoped>
.export-page{display:grid;grid-template-rows:72px minmax(0,1fr);gap:10px}.export-head{display:flex;align-items:center;justify-content:space-between;padding:0 6px}.ready{display:grid;grid-template-columns:24px auto;gap:1px 9px;align-items:center}.ready .status-symbol{grid-row:1/3;width:22px;height:22px;border-radius:50%;display:grid;place-items:center;color:white;background:var(--orange)}.ready.success .status-symbol{background:var(--green)}.ready.working .status-symbol{background:var(--blue)}.ready b{font-size:15px}.ready small{color:#697d9f;max-width:610px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.export-layout{min-height:0;display:grid;grid-template-columns:minmax(620px,1.6fr) minmax(420px,1fr);gap:14px}.export-left{min-height:0;display:grid;grid-template-rows:minmax(300px,1fr) 126px 170px 137px;gap:10px}.full-preview{overflow:hidden;display:flex;flex-direction:column}.export-video{flex:1;position:relative;min-height:0}.export-video>span,.export-video>time{position:absolute;top:12px;color:#fff;background:rgba(4,15,31,.79);padding:6px 10px;border-radius:5px;font-size:13px}.export-video>span{left:13px}.export-video>time{right:13px}.caption{position:absolute;left:50%;transform:translateX(-50%);bottom:15px;color:white;background:rgba(4,15,31,.76);padding:7px 14px;border-radius:5px;font-size:18px;white-space:nowrap}.player{height:51px;display:flex;align-items:center;gap:13px;padding:0 18px}.player button{border:0;background:transparent;color:var(--text);display:grid;place-items:center}.native-range{accent-color:var(--blue);height:7px;min-width:0}.scrub-range{flex:1}.volume-range{width:80px}.shot-strip{display:grid;grid-template-columns:repeat(5,1fr);gap:8px;padding:10px 12px;border:1px solid var(--line);border-radius:9px;background:#fff}.shot-strip article{padding:4px;border-radius:7px;cursor:pointer}.shot-strip article.active{background:#edf5ff}.shot-strip article>div{height:78px;border-radius:6px;position:relative;border:2px solid transparent}.shot-strip article.active>div{border-color:var(--blue)}.shot-strip b{position:absolute;top:5px;left:5px;color:#fff;background:#06182f;padding:3px 5px;border-radius:4px}.shot-strip time{position:absolute;right:4px;bottom:4px;color:#fff;background:#06182f;padding:3px;font-size:11px}.shot-strip article>span{display:block;margin-top:4px;font-size:12px}.mix-panel{overflow:hidden}.mix-panel .panel-head{height:48px}.mix-panel .panel-head .btn{min-height:34px}.mix-row{height:29px;display:grid;grid-template-columns:30px 55px 1fr 50px 30px 150px;align-items:center;gap:8px;padding:0 24px}.mix-icon{font-size:22px;color:#274b80}.mix-row label{font-size:12px}.mix-row input{accent-color:var(--blue)}.mute-button{border:0;background:transparent;display:grid;place-items:center;color:#37537d}.final-timeline .panel-head{height:42px}.final-timeline .panel-head span{font-size:12px;color:#5b7094}.final-track{height:48px;padding:7px 18px 0;display:flex;position:relative}.final-track i{flex:1;display:flex;align-items:center;justify-content:center;gap:3px;font-style:normal;background:#cfe2ff;border-right:2px solid white}.final-track i:nth-child(2){background:#d8f2e8}.final-track i.notReady{color:#a75416;background:#ffebc6}.marker{position:absolute;top:0;bottom:-24px;width:2px;background:#ff394e}.marker:before{content:"";position:absolute;top:0;left:-4px;border-left:5px solid transparent;border-right:5px solid transparent;border-top:8px solid #ff394e}.ticks{display:flex;justify-content:space-between;padding:0 18px;font-size:11px;color:#637797}.export-right{min-height:0;display:grid;grid-template-rows:241px 183px minmax(0,1fr);gap:12px}.checklist{overflow:hidden}.checklist .panel-head h2{display:flex;align-items:center;gap:9px}.check-big,.check{border-radius:50%;display:grid;place-items:center;color:#fff;background:var(--green)}.check-big{width:25px;height:25px}.check-big.failed,.checklist p.failed .check{background:#f0644c}.checklist p{height:45px;display:grid;grid-template-columns:34px 1fr auto;align-items:center;padding:0 20px;border-bottom:1px solid #e5ebf4}.checklist p>span:last-child{color:#6b7f9f;font-size:12px}.checklist>small{display:block;padding:6px 20px;color:#7083a1}.export-info .panel-head{gap:10px}.capability-badge{font-size:11px;color:#c34f15;background:#fff0e7;padding:5px 8px;border-radius:5px}.capability-badge.available{color:#078c57;background:#e8f8f1}.export-info>div:last-child{height:132px;display:grid;grid-template-columns:repeat(3,1fr);padding:18px 14px}.export-info article{display:flex;flex-direction:column;align-items:center;gap:4px;border-right:1px solid #dce5f0}.export-info article:last-child{border:0}.export-info svg{color:var(--blue)}.export-info article span,.export-info article small{font-size:11px;color:#5d7295}.export-info article b{font-size:17px}.export-settings{min-height:0;display:flex;flex-direction:column;padding-bottom:12px}.export-settings .panel-head .btn{min-height:34px}.settings-list{padding:9px 14px;display:flex;flex-direction:column;gap:6px}.settings-list label{height:33px;display:grid;grid-template-columns:28px 95px 1fr;align-items:center}.settings-list select,.settings-list input{height:32px;text-align:left;padding:0 10px;border:1px solid #d2deee;border-radius:6px;background:#fff;font-size:12px;min-width:0}.settings-list .path-row{grid-template-columns:28px 95px minmax(0,1fr) 52px;gap:4px}.path-row button{height:32px;border:1px solid #d2deee;border-radius:6px;background:#f3f6fa;color:#8a9ab1}.export-btn{margin:4px 14px 8px;min-height:48px;font-size:17px}.export-btn:disabled{opacity:.65;cursor:wait}.export-state{margin:0 14px;padding:8px 10px;border-radius:7px;background:#fff1e9;color:#8f3f18;display:flex;flex-direction:column;gap:3px}.export-state.success{background:#e8f8f1;color:#087d51}.export-state.working{background:#eaf3ff;color:#0a5fd5}.export-state span{font-size:11px;line-height:1.35}.export-settings>small{margin:7px 14px;color:#6e82a1}@media(max-width:1380px){.export-left{grid-template-rows:minmax(250px,1fr) 110px 150px 120px}.export-right{grid-template-rows:220px 165px minmax(0,1fr)}.shot-strip article>div{height:64px}.mix-row{padding:0 14px}.settings-list{gap:3px;padding-top:5px}.settings-list label{height:30px}.export-btn{min-height:40px;margin-bottom:5px}.checklist p{height:39px}.export-state{padding:5px 8px}.export-settings>small{margin-top:4px}}
.export-video video{width:100%;height:100%;display:block;object-fit:contain;background:#07101f}.empty-video{position:absolute;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:8px;color:#8ea0bc;background:#0b1728}.empty-video b{color:#dfe8f5;font-size:17px}.empty-video span{position:static;color:#9fb0c8;background:transparent;padding:0}.shot-strip video{width:100%;height:100%;display:block;object-fit:cover;border-radius:4px}.missing-thumb{position:absolute;inset:0;display:grid;place-items:center;color:#7184a3;background:#edf3fa}.shot-strip article>div>b,.shot-strip article>div>time{z-index:2}
</style>
