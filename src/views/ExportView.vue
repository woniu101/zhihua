<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { AlertTriangle, Check, Clock3, FileVideo2, FolderOpen, Maximize2, Monitor, Music2, Pause, Play, RotateCw, Subtitles, Upload, Volume2, VolumeX, X } from "lucide-vue-next";
import { shots } from "../data/demo";
import {
  defaultExportSettings,
  estimateOutputSizeMb,
  inspectExportCapability,
  inspectIntegrity,
  requestNativeExport,
  type ExportCapability,
  type ExportStatus,
} from "../services/exportService";

interface MixTrack { name: string; icon: string; volume: number; fade: boolean; muted: boolean }

const settings = reactive(defaultExportSettings());
const mixTracks = reactive<MixTrack[]>([
  { name: "旁白", icon: "♩", volume: 80, fade: true, muted: false },
  { name: "音乐", icon: "♫", volume: 60, fade: true, muted: false },
  { name: "环境音", icon: "≋", volume: 50, fade: true, muted: false },
]);
const capability = ref<ExportCapability>({ available: false, label: "正在检测 FFmpeg", reason: "正在读取桌面导出能力。" });
const exportStatus = ref<ExportStatus>("checking");
const exportMessage = ref("正在执行完整性检查和 FFmpeg 能力检测。");
const lastCheckedAt = ref("");
const playing = ref(false);
const playhead = ref(18.4);
const previewVolume = ref(62);
const selectedShotId = ref("03");
const totalDuration = 42;

const integrityItems = computed(() => inspectIntegrity({
  totalShots: shots.length,
  readyShotIds: shots.filter((shot) => shot.status === "1080p 就绪").map((shot) => shot.id),
  narrationComplete: shots.every((shot) => Boolean(shot.text.trim())),
  subtitleComplete: shots.every((shot) => Boolean(shot.text.trim())),
  sourceRecordsComplete: true,
  missingAssetNames: [],
}));
const integrityPassed = computed(() => integrityItems.value.every((item) => item.passed));
const readyCount = computed(() => integrityItems.value.find((item) => item.id === "shots")?.detail ?? "0/0 就绪");
const estimatedSize = computed(() => estimateOutputSizeMb(totalDuration, settings.frameRate));
const playheadText = computed(() => `00:${playhead.value.toFixed(1).padStart(4, "0")}`);
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
  } catch {
    localStorage.removeItem("zhihua.export.settings");
    localStorage.removeItem("zhihua.export.mix");
  }
}
function resetMix() {
  mixTracks.splice(0, mixTracks.length,
    { name: "旁白", icon: "♩", volume: 80, fade: true, muted: false },
    { name: "音乐", icon: "♫", volume: 60, fade: true, muted: false },
    { name: "环境音", icon: "≋", volume: 50, fade: true, muted: false },
  );
}
function resetSettings() {
  Object.assign(settings, defaultExportSettings());
}
async function runChecks() {
  exportStatus.value = "checking";
  exportMessage.value = "正在重新检查分镜、素材和桌面导出能力。";
  capability.value = await inspectExportCapability();
  lastCheckedAt.value = new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }).format(new Date());
  if (!integrityPassed.value) {
    exportStatus.value = "blocked";
    exportMessage.value = `当前仅 ${readyCount.value}，请先在分镜页为全部镜头制作正式 1080p 成片。`;
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
    await requestNativeExport({ ...settings });
    exportStatus.value = "succeeded";
    exportMessage.value = "导出完成。";
  } catch (error) {
    exportStatus.value = "failed";
    exportMessage.value = error instanceof Error ? error.message : "导出失败。";
  }
}

watch(settings, savePreferences, { deep: true });
watch(mixTracks, savePreferences, { deep: true });
onMounted(async () => { restorePreferences(); await runChecks(); });
</script>

<template>
  <section class="page export-page">
    <header class="export-head">
      <div class="page-title-line"><h1>检查并导出</h1><span class="ready" :class="statusTone"><span class="status-symbol"><Check v-if="integrityPassed && capability.available" :size="14"/><AlertTriangle v-else :size="15"/></span><b>{{ exportStatus==='checking' ? '正在检查' : exportStatus==='blocked' ? '暂不可导出' : exportStatus==='exporting' ? '正在导出' : exportStatus==='failed' ? '导出失败' : exportStatus==='succeeded' ? '导出完成' : '准备就绪' }}</b><small>{{ exportMessage }}</small></span></div>
      <button class="btn" @click="runChecks"><RotateCw :size="17"/>重新检查</button>
    </header>
    <div class="export-layout">
      <section class="export-left">
        <div class="panel full-preview"><div class="export-video lightning-image"><span>1920 × 1080　16:9</span><time>{{ playheadText }} / 00:42.0</time><div class="caption">当电场足够强，空气就会被击穿。</div></div><div class="player"><button aria-label="播放或暂停" @click="playing=!playing"><Pause v-if="playing" :size="20" fill="currentColor"/><Play v-else :size="20" fill="currentColor"/></button><span>Ⅰ◀</span><span>▶Ⅰ</span><b>{{ playheadText }}</b><span>/ 00:42.0</span><input v-model.number="playhead" class="native-range scrub-range" type="range" min="0" :max="totalDuration" step="0.1"/><Volume2 :size="19"/><input v-model.number="previewVolume" aria-label="预览音量" class="native-range volume-range" type="range" min="0" max="100"/><Maximize2 :size="18"/></div></div>
        <div class="shot-strip"><article v-for="shot in shots" :key="shot.id" :class="{active:selectedShotId===shot.id}" @click="selectedShotId=shot.id"><div class="lightning-image"><b>{{ shot.id }}</b><time>00:{{ shot.duration.replace('秒','').padStart(2,'0') }}</time></div><span>{{ shot.title }}</span></article></div>
        <div class="panel mix-panel"><div class="panel-head"><h2>音频混音</h2><button class="btn" @click="resetMix">恢复默认</button></div><div class="mix-row" v-for="track in mixTracks" :key="track.name"><span class="mix-icon">{{ track.icon }}</span><b>{{ track.name }}</b><input v-model.number="track.volume" :aria-label="`${track.name}音量`" class="native-range" type="range" min="0" max="100" :disabled="track.muted"/><span>{{ track.muted ? '静音' : `${track.volume}%` }}</span><button class="mute-button" :aria-label="`${track.name}${track.muted?'取消静音':'静音'}`" @click="track.muted=!track.muted"><VolumeX v-if="track.muted" :size="17"/><Volume2 v-else :size="17"/></button><label><input v-model="track.fade" type="checkbox"/> 应用淡入淡出</label></div></div>
        <div class="panel final-timeline"><div class="panel-head"><h3>最终检查时间线</h3><span>{{ playheadText }}　　　　　　　　　总时长 00:42.0</span></div><div class="final-track"><i v-for="shot in shots" :key="shot.id" :class="{notReady:shot.status!=='1080p 就绪'}">{{ shot.id }}<X v-if="shot.status!=='1080p 就绪'" :size="12"/></i><span class="marker" :style="{left:`${playhead/totalDuration*100}%`}"></span></div><div class="ticks"><span>0:00</span><span>0:10</span><span>0:20</span><span>0:30</span><span>0:42</span></div></div>
      </section>

      <aside class="export-right">
        <section class="panel checklist"><div class="panel-head"><h2><span class="check-big" :class="{failed:!integrityPassed}"><Check v-if="integrityPassed" :size="17"/><AlertTriangle v-else :size="16"/></span>完整性检查</h2><b :class="integrityPassed?'success-text':'warning-text'">{{ integrityPassed ? '全部通过' : '存在阻塞项' }}</b></div><p v-for="item in integrityItems" :key="item.id" :class="{failed:!item.passed}"><span class="check"><Check v-if="item.passed" :size="15"/><X v-else :size="15"/></span><b>{{ item.label }}</b><span>{{ item.detail }}</span></p><small v-if="lastCheckedAt">最近检查 {{ lastCheckedAt }}</small></section>
        <section class="panel export-info"><div class="panel-head"><h2>导出信息</h2><b class="capability-badge" :class="{available:capability.available}">{{ capability.label }}</b></div><div><article><Monitor :size="31"/><span>分辨率</span><b>{{ settings.resolution }}</b><small>{{ settings.ratio }}</small></article><article><Clock3 :size="31"/><span>总时长</span><b>{{ totalDuration }} 秒</b></article><article><FileVideo2 :size="31"/><span>预计文件大小</span><b>约 {{ estimatedSize }} MB</b></article></div></section>
        <section class="panel export-settings"><div class="panel-head"><h2>导出设置</h2><button class="btn" @click="resetSettings">恢复默认</button></div><div class="settings-list"><label><FileVideo2 :size="18"/><span>导出格式</span><select v-model="settings.videoCodec"><option value="H.264">MP4 · H.264（通用，推荐）</option></select></label><label><Music2 :size="18"/><span>音频编码</span><select v-model="settings.audioCodec"><option value="AAC">AAC（高质量）</option></select></label><label><Monitor :size="18"/><span>帧率</span><select v-model.number="settings.frameRate"><option :value="24">24 fps（电影感）</option><option :value="25">25 fps</option><option :value="30">30 fps（更流畅）</option></select></label><label><Monitor :size="18"/><span>画面比例</span><select v-model="settings.ratio"><option value="16:9">16:9（1920 × 1080）</option></select></label><label><Subtitles :size="18"/><span>字幕处理</span><select v-model="settings.subtitleMode"><option value="burn-and-srt">嵌入画面并保存 SRT</option><option value="burn">仅嵌入画面</option><option value="srt">仅保存 SRT</option></select></label><label class="path-row"><FolderOpen :size="18"/><span>保存位置</span><input v-model.trim="settings.outputDirectory" aria-label="导出保存位置"/><button disabled title="目录选择命令尚未接入">浏览</button></label></div><button class="btn primary export-btn" :disabled="exportStatus==='checking'||exportStatus==='exporting'" @click="beginExport"><Upload :size="20"/>{{ exportStatus==='exporting' ? '正在导出' : '导出 MP4' }}</button><div class="export-state" :class="statusTone"><b>{{ capability.available ? (integrityPassed ? '可执行导出' : '等待分镜就绪') : 'FFmpeg native command 未接入' }}</b><span>{{ capability.available ? exportMessage : capability.reason }}</span></div><small>导出参数和音量设置会自动保存在本机。</small></section>
      </aside>
    </div>
  </section>
</template>

<style scoped>
.export-page{display:grid;grid-template-rows:72px minmax(0,1fr);gap:10px}.export-head{display:flex;align-items:center;justify-content:space-between;padding:0 6px}.ready{display:grid;grid-template-columns:24px auto;gap:1px 9px;align-items:center}.ready .status-symbol{grid-row:1/3;width:22px;height:22px;border-radius:50%;display:grid;place-items:center;color:white;background:var(--orange)}.ready.success .status-symbol{background:var(--green)}.ready.working .status-symbol{background:var(--blue)}.ready b{font-size:15px}.ready small{color:#697d9f;max-width:610px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.export-layout{min-height:0;display:grid;grid-template-columns:minmax(620px,1.6fr) minmax(420px,1fr);gap:14px}.export-left{min-height:0;display:grid;grid-template-rows:minmax(300px,1fr) 126px 170px 137px;gap:10px}.full-preview{overflow:hidden;display:flex;flex-direction:column}.export-video{flex:1;position:relative;min-height:0}.export-video>span,.export-video>time{position:absolute;top:12px;color:#fff;background:rgba(4,15,31,.79);padding:6px 10px;border-radius:5px;font-size:13px}.export-video>span{left:13px}.export-video>time{right:13px}.caption{position:absolute;left:50%;transform:translateX(-50%);bottom:15px;color:white;background:rgba(4,15,31,.76);padding:7px 14px;border-radius:5px;font-size:18px;white-space:nowrap}.player{height:51px;display:flex;align-items:center;gap:13px;padding:0 18px}.player button{border:0;background:transparent;color:var(--text);display:grid;place-items:center}.native-range{accent-color:var(--blue);height:7px;min-width:0}.scrub-range{flex:1}.volume-range{width:80px}.shot-strip{display:grid;grid-template-columns:repeat(5,1fr);gap:8px;padding:10px 12px;border:1px solid var(--line);border-radius:9px;background:#fff}.shot-strip article{padding:4px;border-radius:7px;cursor:pointer}.shot-strip article.active{background:#edf5ff}.shot-strip article>div{height:78px;border-radius:6px;position:relative;border:2px solid transparent}.shot-strip article.active>div{border-color:var(--blue)}.shot-strip b{position:absolute;top:5px;left:5px;color:#fff;background:#06182f;padding:3px 5px;border-radius:4px}.shot-strip time{position:absolute;right:4px;bottom:4px;color:#fff;background:#06182f;padding:3px;font-size:11px}.shot-strip article>span{display:block;margin-top:4px;font-size:12px}.mix-panel{overflow:hidden}.mix-panel .panel-head{height:48px}.mix-panel .panel-head .btn{min-height:34px}.mix-row{height:29px;display:grid;grid-template-columns:30px 55px 1fr 50px 30px 150px;align-items:center;gap:8px;padding:0 24px}.mix-icon{font-size:22px;color:#274b80}.mix-row label{font-size:12px}.mix-row input{accent-color:var(--blue)}.mute-button{border:0;background:transparent;display:grid;place-items:center;color:#37537d}.final-timeline .panel-head{height:42px}.final-timeline .panel-head span{font-size:12px;color:#5b7094}.final-track{height:48px;padding:7px 18px 0;display:flex;position:relative}.final-track i{flex:1;display:flex;align-items:center;justify-content:center;gap:3px;font-style:normal;background:#cfe2ff;border-right:2px solid white}.final-track i:nth-child(2){background:#d8f2e8}.final-track i.notReady{color:#a75416;background:#ffebc6}.marker{position:absolute;top:0;bottom:-24px;width:2px;background:#ff394e}.marker:before{content:"";position:absolute;top:0;left:-4px;border-left:5px solid transparent;border-right:5px solid transparent;border-top:8px solid #ff394e}.ticks{display:flex;justify-content:space-between;padding:0 18px;font-size:11px;color:#637797}.export-right{min-height:0;display:grid;grid-template-rows:241px 183px minmax(0,1fr);gap:12px}.checklist{overflow:hidden}.checklist .panel-head h2{display:flex;align-items:center;gap:9px}.check-big,.check{border-radius:50%;display:grid;place-items:center;color:#fff;background:var(--green)}.check-big{width:25px;height:25px}.check-big.failed,.checklist p.failed .check{background:#f0644c}.checklist p{height:45px;display:grid;grid-template-columns:34px 1fr auto;align-items:center;padding:0 20px;border-bottom:1px solid #e5ebf4}.checklist p>span:last-child{color:#6b7f9f;font-size:12px}.checklist>small{display:block;padding:6px 20px;color:#7083a1}.export-info .panel-head{gap:10px}.capability-badge{font-size:11px;color:#c34f15;background:#fff0e7;padding:5px 8px;border-radius:5px}.capability-badge.available{color:#078c57;background:#e8f8f1}.export-info>div:last-child{height:132px;display:grid;grid-template-columns:repeat(3,1fr);padding:18px 14px}.export-info article{display:flex;flex-direction:column;align-items:center;gap:4px;border-right:1px solid #dce5f0}.export-info article:last-child{border:0}.export-info svg{color:var(--blue)}.export-info article span,.export-info article small{font-size:11px;color:#5d7295}.export-info article b{font-size:17px}.export-settings{min-height:0;display:flex;flex-direction:column;padding-bottom:12px}.export-settings .panel-head .btn{min-height:34px}.settings-list{padding:9px 14px;display:flex;flex-direction:column;gap:6px}.settings-list label{height:33px;display:grid;grid-template-columns:28px 95px 1fr;align-items:center}.settings-list select,.settings-list input{height:32px;text-align:left;padding:0 10px;border:1px solid #d2deee;border-radius:6px;background:#fff;font-size:12px;min-width:0}.settings-list .path-row{grid-template-columns:28px 95px minmax(0,1fr) 52px;gap:4px}.path-row button{height:32px;border:1px solid #d2deee;border-radius:6px;background:#f3f6fa;color:#8a9ab1}.export-btn{margin:4px 14px 8px;min-height:48px;font-size:17px}.export-btn:disabled{opacity:.65;cursor:wait}.export-state{margin:0 14px;padding:8px 10px;border-radius:7px;background:#fff1e9;color:#8f3f18;display:flex;flex-direction:column;gap:3px}.export-state.success{background:#e8f8f1;color:#087d51}.export-state.working{background:#eaf3ff;color:#0a5fd5}.export-state span{font-size:11px;line-height:1.35}.export-settings>small{margin:7px 14px;color:#6e82a1}@media(max-width:1380px){.export-left{grid-template-rows:minmax(250px,1fr) 110px 150px 120px}.export-right{grid-template-rows:220px 165px minmax(0,1fr)}.shot-strip article>div{height:64px}.mix-row{padding:0 14px}.settings-list{gap:3px;padding-top:5px}.settings-list label{height:30px}.export-btn{min-height:40px;margin-bottom:5px}.checklist p{height:39px}.export-state{padding:5px 8px}.export-settings>small{margin-top:4px}}
</style>
