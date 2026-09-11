<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ArrowDown, ArrowUp, ChevronDown, Copy, Film, LoaderCircle, Maximize2, Music2, Pause, Play, Plus, RotateCcw, Sparkles, Star, Subtitles, Trash2, Volume2, X } from "lucide-vue-next";
import type { GenerationJob } from "../domain/providers";
import type { AspectRatio, CandidateQuality, GenerationMode } from "../domain/storyboard";
import { frameProfile } from "../domain/frameProfiles";
import type { AssetItem, AssetMediaType } from "../domain/assets";
import { assetRepository } from "../services/assetRepository";
import { generationRepository, type CandidateVersion, type EnhancedVersion } from "../services/generationRepository";
import { ComfyUiH3Provider, normalizeConnectionFailure, serviceRepository } from "../services/serviceRepository";
import { ttsRepository, type NarrationArtifact, type SystemVoice } from "../services/ttsRepository";
import { requestNativePreview } from "../services/exportService";
import { useStoryboardStore } from "../stores/storyboard";

const { scenes, settings, selectedScene, selectedSceneId, loading, loadError, select, update, save, updateSelected, add, duplicateSelected, removeSelected, moveSelected } = useStoryboardStore();
const provider = new ComfyUiH3Provider();
const task = ref<GenerationJob>();
const referenceAssets = ref<AssetItem[]>([]);
const candidateVersions = ref<CandidateVersion[]>([]);
const enhancedVersions = ref<EnhancedVersion[]>([]);
const previewVersionId = ref("");
const previewEnhancedId = ref("");
const taskError = ref("");
const submitting = ref(false);
const batchSubmitting = ref(false);
const batchNotice = ref("");
const previewBusy = ref(false);
const fullPreviewUrl = ref("");
const fullPreviewOpen = ref(false);
const enhancing = ref(false);
const taskKind = ref<"candidate" | "enhancement">("candidate");
const systemVoices = ref<SystemVoice[]>([]);
const selectedVoiceId = ref("");
const narrationArtifact = ref<NarrationArtifact>();
const narrationBusy = ref(false);
const narrationError = ref("");
const selectedNarrationAssetId = ref("");
let narrationAudio: HTMLAudioElement | undefined;
const pollTimers = new Map<string, number>();
const pollRetryDelays = new Map<string, number>();
const pollingJobs = new Set<string>();
let recoveredProjectId = "";
const modes: Array<{ id: GenerationMode; label: string; symbol: string }> = [
  { id: "t2v", label: "自由生成", symbol: "✦" },
  { id: "i2v", label: "从这张画面开始", symbol: "▧" },
  { id: "flf2v", label: "首尾画面过渡", symbol: "→" },
  { id: "r2v", label: "保持角色与场景", symbol: "◎" },
];
const statusLabel = { draft: "待生成", ready: "已就绪", generating: "生成中", generated: "候选版本", approved: "正式版本", failed: "生成失败" } as const;
const statusTone = { draft: "muted", ready: "success", generating: "warning", generated: "primary", approved: "success", failed: "warning" } as const;
const durationLabel = computed(() => `${Math.round((selectedScene.value?.targetDurationMs ?? 5000) / 1000)}秒`);
const setDuration = (seconds: 5 | 10 | 15) => updateSelected({ targetDurationMs: (seconds * 1000) as 5000 | 10000 | 15000 });
const setQuality = (quality: CandidateQuality) => updateSelected({ quality });
const taskPercent = computed(() => Math.max(0, Math.min(100, Math.round((task.value?.progress ?? 0) * 100))));
const taskStatusLabel = computed(() => {
  const status = task.value?.status;
  if (!status) return "空闲";
  return {
    queued: "已排队",
    waiting_for_compute: "等待算力",
    preparing: "准备环境",
    uploading: "上传素材",
    running: "生成中",
    upscaling: "制作 1080p 增强版",
    downloading: "下载结果",
    completed: "已完成",
    failed: "失败",
    cancelled: "已取消",
    interrupted: "已中断",
  }[status];
});
const taskDescription = computed(() => {
  if (taskError.value) return taskError.value;
  if (!task.value) return "本地编辑不会启动 GPU";
  if (task.value.status === "queued") return "任务已安全保存，等待生成执行器";
  return task.value.stageMessage;
});

const selectedReferenceIds = computed(() => selectedScene.value?.assetIds ?? []);
const imageAssets = computed(() => referenceAssets.value.filter((asset) => asset.mediaType === "image"));
const videoAssets = computed(() => referenceAssets.value.filter((asset) => asset.mediaType === "video"));
const audioAssets = computed(() => referenceAssets.value.filter((asset) => asset.mediaType === "audio"));
const latestCandidate = <T,>(items: T[]): T | undefined => items[items.length - 1];
const previewVersion = computed(() => candidateVersions.value.find((item) => item.id === previewVersionId.value) ?? latestCandidate(candidateVersions.value));
const enhancedForOfficial = computed(() => latestCandidate(enhancedVersions.value.filter((item) => item.sourceCandidateId === selectedScene.value?.selectedVersionId)));
const previewEnhanced = computed(() => enhancedVersions.value.find((item) => item.id === previewEnhancedId.value));
const previewMedia = computed(() => previewEnhanced.value ?? previewVersion.value);
const activeFrameProfile = computed(() => frameProfile(settings.value.aspectRatio));
const canSelectOfficial = computed(() => Boolean(
  previewVersion.value
  && previewVersion.value.aspectRatio === activeFrameProfile.value.aspectRatio
  && selectedScene.value?.selectedVersionId !== previewVersion.value.id,
));
const isTaskActive = computed(() => Boolean(task.value && !["completed", "failed", "cancelled", "interrupted"].includes(task.value.status)));
const canMakeEnhancement = computed(() => Boolean(
  selectedScene.value?.selectedVersionId
  && settings.value.aspectRatio === "16:9"
  && !submitting.value
  && !enhancing.value
  && !isTaskActive.value,
));

function setProjectAspect(event: Event) {
  settings.value.aspectRatio = (event.target as HTMLSelectElement).value as AspectRatio;
  previewEnhancedId.value = "";
}

async function ensureSystemVoices() {
  if (systemVoices.value.length) return;
  try {
    systemVoices.value = await ttsRepository.listVoices();
    selectedVoiceId.value ||= systemVoices.value.find((voice) => voice.locale.toLowerCase().startsWith("zh"))?.id
      ?? systemVoices.value[0]?.id
      ?? "";
  } catch (error) {
    narrationError.value = normalizeConnectionFailure(error).message;
  }
}

async function loadNarration(projectId?: string, sceneId?: string) {
  narrationAudio?.pause();
  narrationArtifact.value = undefined;
  selectedNarrationAssetId.value = "";
  if (!projectId || !sceneId) return;
  await ensureSystemVoices();
  narrationArtifact.value = await ttsRepository.get(projectId, sceneId).catch(() => undefined);
  if (narrationArtifact.value?.voiceId.startsWith("imported:")) {
    selectedNarrationAssetId.value = narrationArtifact.value.voiceId.slice("imported:".length);
  } else if (narrationArtifact.value?.voiceId && systemVoices.value.some((voice) => voice.id === narrationArtifact.value?.voiceId)) {
    selectedVoiceId.value = narrationArtifact.value.voiceId;
    selectedNarrationAssetId.value = "";
  }
}

async function synthesizeNarration(playAfter = false) {
  const scene = selectedScene.value;
  if (!scene || !scene.narration.trim()) {
    narrationError.value = "请先填写旁白文案";
    return;
  }
  await ensureSystemVoices();
  if (!selectedVoiceId.value) {
    narrationError.value = "Windows 没有可用的系统语音";
    return;
  }
  narrationBusy.value = true;
  narrationError.value = "";
  try {
    await save(scene.id, { narration: scene.narration });
    narrationArtifact.value = await ttsRepository.synthesize({
      projectId: scene.projectId,
      sceneId: scene.id,
      voiceId: selectedVoiceId.value,
      rate: 0,
      volume: 100,
    });
    selectedNarrationAssetId.value = "";
    if (playAfter) playNarration();
  } catch (error) {
    narrationError.value = normalizeConnectionFailure(error).message;
  } finally {
    narrationBusy.value = false;
  }
}

async function importNarrationAudio() {
  const scene = selectedScene.value;
  if (!scene || !selectedNarrationAssetId.value) {
    narrationError.value = "请先选择一段已导入素材库的录音";
    return;
  }
  if (!scene.narration.trim()) {
    narrationError.value = "请先填写与录音对应的旁白文案";
    return;
  }
  narrationBusy.value = true;
  narrationError.value = "";
  try {
    await save(scene.id, { narration: scene.narration });
    narrationArtifact.value = await ttsRepository.importAudio(
      scene.projectId,
      scene.id,
      selectedNarrationAssetId.value,
    );
  } catch (error) {
    narrationError.value = normalizeConnectionFailure(error).message;
  } finally {
    narrationBusy.value = false;
  }
}

function playNarration() {
  if (!narrationArtifact.value) {
    void synthesizeNarration(true);
    return;
  }
  narrationAudio?.pause();
  narrationAudio = new Audio(narrationArtifact.value.previewUrl);
  void narrationAudio.play().catch((error) => {
    narrationError.value = error instanceof Error ? error.message : "旁白预览失败";
  });
}

function updateNarrationText(event: Event) {
  narrationAudio?.pause();
  narrationArtifact.value = undefined;
  narrationError.value = "旁白文案已修改，请重新生成旁白。";
  updateSelected({ narration: (event.target as HTMLTextAreaElement).value });
}

async function loadReferenceAssets(projectId?: string) {
  if (!projectId) {
    referenceAssets.value = [];
    return;
  }
  try {
    referenceAssets.value = await assetRepository.list(projectId);
  } catch {
    referenceAssets.value = [];
  }
}

async function loadCandidateVersions(projectId?: string, sceneId?: string) {
  if (!projectId || !sceneId) {
    candidateVersions.value = [];
    enhancedVersions.value = [];
    previewVersionId.value = "";
    previewEnhancedId.value = "";
    return;
  }
  candidateVersions.value = await generationRepository.list(projectId, sceneId).catch(() => []);
  enhancedVersions.value = await generationRepository.listEnhanced(projectId, sceneId).catch(() => []);
  previewVersionId.value = selectedScene.value?.selectedVersionId
    ?? latestCandidate(candidateVersions.value)?.id
    ?? "";
  previewEnhancedId.value = enhancedForOfficial.value?.id ?? "";
}

function setReferenceSlot(slot: number, mediaType: AssetMediaType, event: Event) {
  const value = (event.target as HTMLSelectElement).value;
  const scene = selectedScene.value;
  if (!scene) return;
  const ids = [...scene.assetIds];
  const asset = referenceAssets.value.find((item) => item.id === value && item.mediaType === mediaType);
  if (asset) ids[slot] = asset.id;
  else if (slot === 0) ids.splice(0);
  else ids.splice(slot, 1);
  updateSelected({ assetIds: ids.filter(Boolean) });
}

function chooseMode(mode: GenerationMode) {
  if (selectedScene.value?.generationMode === mode) return;
  updateSelected({ generationMode: mode, assetIds: [] });
}

function clearPoll(jobId?: string) {
  if (jobId) {
    const timer = pollTimers.get(jobId);
    if (timer !== undefined) window.clearTimeout(timer);
    pollTimers.delete(jobId);
    return;
  }
  pollTimers.forEach((timer) => window.clearTimeout(timer));
  pollTimers.clear();
  pollRetryDelays.clear();
}

function schedulePoll(jobId: string, callback: () => void, delay = 2500) {
  const current = pollTimers.get(jobId);
  if (current !== undefined) window.clearTimeout(current);
  pollTimers.set(jobId, window.setTimeout(callback, delay));
}

function sceneStatusFor(job: GenerationJob) {
  if (job.status === "completed") return "generated" as const;
  if (job.status === "cancelled") return "draft" as const;
  if (["failed", "interrupted"].includes(job.status)) return "failed" as const;
  return "generating" as const;
}

async function refreshTask(sceneId: string, jobId: string) {
  if (pollingJobs.has(jobId)) return;
  pollingJobs.add(jobId);
  clearPoll(jobId);
  if (selectedSceneId.value === sceneId) taskKind.value = "candidate";
  try {
    const latest = await provider.getStatus(jobId);
    pollRetryDelays.set(jobId, 2500);
    taskError.value = "";
    if (selectedSceneId.value === sceneId) task.value = latest;
    const scene = scenes.value.find((item) => item.id === sceneId);
    if (latest.status === "completed" && scene) {
      const existing = candidateVersions.value.some((item) => item.jobId === jobId);
      if (!existing) {
        if (selectedSceneId.value === sceneId) {
          task.value = { ...latest, status: "downloading", progress: null, stageMessage: "正在校验并保存候选视频" };
        }
        const downloaded = await generationRepository.downloadCompletedJob(
          scene.projectId,
          jobId,
          frameProfile(settings.value.aspectRatio),
        );
        if (selectedSceneId.value === sceneId) {
          candidateVersions.value = await generationRepository.list(scene.projectId, sceneId);
          previewVersionId.value = latestCandidate(downloaded)?.id ?? latestCandidate(candidateVersions.value)?.id ?? "";
          task.value = latest;
        }
      }
      update(sceneId, { status: "generated", generationStage: "候选视频已保存到本地" });
    } else {
      update(sceneId, {
        status: sceneStatusFor(latest),
        generationStage: latest.stageMessage,
      });
    }
    if (!["completed", "failed", "cancelled", "interrupted"].includes(latest.status)) {
      schedulePoll(jobId, () => void refreshTask(sceneId, jobId));
    } else {
      pollRetryDelays.delete(jobId);
    }
  } catch (error) {
    const failure = normalizeConnectionFailure(error);
    if (selectedSceneId.value === sceneId) {
      taskError.value = `安全连接暂时中断，远端任务仍会继续；正在恢复：${failure.message}`;
    }
    update(sceneId, { status: "generating", generationStage: "安全连接正在恢复，远端任务继续运行" });
    const delay = Math.min((pollRetryDelays.get(jobId) ?? 2500) * 2, 20_000);
    pollRetryDelays.set(jobId, delay);
    schedulePoll(jobId, () => void refreshTask(sceneId, jobId), delay);
  } finally {
    pollingJobs.delete(jobId);
  }
}

async function refreshEnhancementTask(sceneId: string, jobId: string, sourceCandidateId: string) {
  if (pollingJobs.has(jobId)) return;
  pollingJobs.add(jobId);
  clearPoll(jobId);
  if (selectedSceneId.value === sceneId) taskKind.value = "enhancement";
  try {
    const latest = await provider.getStatus(jobId);
    pollRetryDelays.set(jobId, 2500);
    taskError.value = "";
    if (selectedSceneId.value === sceneId) task.value = latest;
    const scene = scenes.value.find((item) => item.id === sceneId);
    if (latest.status === "completed" && scene) {
      const existing = enhancedVersions.value.some((item) => item.jobId === jobId);
      if (!existing) {
        if (selectedSceneId.value === sceneId) {
          task.value = { ...latest, status: "downloading", progress: null, stageMessage: "正在校验并保存 1080p 增强版" };
        }
        const downloaded = await generationRepository.downloadCompletedEnhancement(
          scene.projectId,
          jobId,
          sourceCandidateId,
        );
        if (selectedSceneId.value === sceneId) {
          enhancedVersions.value = await generationRepository.listEnhanced(scene.projectId, sceneId);
          previewEnhancedId.value = latestCandidate(downloaded)?.id ?? enhancedForOfficial.value?.id ?? "";
          task.value = latest;
        }
      }
      update(sceneId, { status: "approved", generationStage: "1080p 增强版已就绪" });
    } else if (scene) {
      const retainsOfficial = ["failed", "cancelled", "interrupted"].includes(latest.status);
      update(sceneId, {
        status: retainsOfficial ? "approved" : "generating",
        generationStage: latest.status === "running" ? "正在制作 1080p 增强版" : latest.stageMessage,
      });
    }
    if (!["completed", "failed", "cancelled", "interrupted"].includes(latest.status)) {
      schedulePoll(
        jobId,
        () => void refreshEnhancementTask(sceneId, jobId, sourceCandidateId),
      );
    } else {
      pollRetryDelays.delete(jobId);
    }
  } catch (error) {
    const failure = normalizeConnectionFailure(error);
    if (selectedSceneId.value === sceneId) {
      taskError.value = `安全连接暂时中断，1080p 任务仍会继续；正在恢复：${failure.message}`;
    }
    update(sceneId, { status: "generating", generationStage: "安全连接正在恢复，1080p 任务继续运行" });
    const delay = Math.min((pollRetryDelays.get(jobId) ?? 2500) * 2, 20_000);
    pollRetryDelays.set(jobId, delay);
    schedulePoll(
      jobId,
      () => void refreshEnhancementTask(sceneId, jobId, sourceCandidateId),
      delay,
    );
  } finally {
    pollingJobs.delete(jobId);
  }
}

async function selectOfficialVersion() {
  const scene = selectedScene.value;
  const version = previewVersion.value;
  if (!scene || !version) return;
  if (version.aspectRatio !== activeFrameProfile.value.aspectRatio) {
    taskError.value = `该候选是 ${version.aspectRatio}，当前项目画幅为 ${activeFrameProfile.value.aspectRatio}，请按当前画幅重新生成。`;
    return;
  }
  try {
    await generationRepository.select(scene.projectId, scene.id, version.id);
    candidateVersions.value = candidateVersions.value.map((item) => ({
      ...item,
      selected: item.id === version.id,
    }));
    await save(scene.id, {
      selectedVersionId: version.id,
      lastUpscaleJobId: undefined,
      status: "approved",
      generationStage: "已设为正式版本",
    });
  } catch (error) {
    taskError.value = normalizeConnectionFailure(error).message;
  }
}

async function generateSelected() {
  const scene = selectedScene.value;
  if (!scene || submitting.value) return;
  taskError.value = "";
  if (!scene.visualPlan.trim()) {
    taskError.value = "请先填写当前分镜的画面描述。";
    return;
  }
  submitting.value = true;
  taskKind.value = "candidate";
  previewEnhancedId.value = "";
  const requestId = scene.pendingRequestId ?? crypto.randomUUID();
  await save(scene.id, { pendingRequestId: requestId, status: "generating", generationStage: "正在提交任务" });
  try {
    const submitted = await provider.submit({
      clientRequestId: requestId,
      projectId: scene.projectId,
      sceneId: scene.id,
      mode: scene.generationMode,
      quality: scene.quality,
      aspectRatio: settings.value.aspectRatio,
      durationSec: (scene.targetDurationMs / 1000) as 5 | 10 | 15,
      prompt: scene.visualPlan,
      narration: scene.narration,
      seed: crypto.getRandomValues(new Uint32Array(1))[0],
      assetIds: scene.assetIds,
      discardH3Audio: settings.value.discardH3Audio,
    });
    task.value = submitted;
    await save(scene.id, {
      lastJobId: submitted.id,
      pendingRequestId: undefined,
      status: sceneStatusFor(submitted),
      generationStage: submitted.stageMessage,
    });
    await refreshTask(scene.id, submitted.id);
  } catch (error) {
    taskError.value = normalizeConnectionFailure(error).message;
    await save(scene.id, { status: "failed", generationStage: taskError.value });
  } finally {
    submitting.value = false;
  }
}

async function generateIncompleteScenes() {
  if (batchSubmitting.value || submitting.value || enhancing.value) return;
  const candidates = scenes.value.filter((scene) =>
    !["generated", "approved", "generating"].includes(scene.status)
  );
  const valid = candidates.filter((scene) => scene.visualPlan.trim());
  if (!valid.length) {
    batchNotice.value = candidates.length
      ? "未完成分镜还没有画面描述，请先补充后再生成。"
      : "全部分镜都已有候选或正式版本。";
    return;
  }

  batchSubmitting.value = true;
  batchNotice.value = `正在将 ${valid.length} 个分镜加入生成队列…`;
  let submittedCount = 0;
  const failures: string[] = [];
  for (const scene of valid) {
    const requestId = scene.pendingRequestId ?? crypto.randomUUID();
    await save(scene.id, {
      pendingRequestId: requestId,
      status: "generating",
      generationStage: "正在批量提交任务",
    });
    try {
      const submitted = await provider.submit({
        clientRequestId: requestId,
        projectId: scene.projectId,
        sceneId: scene.id,
        mode: scene.generationMode,
        quality: scene.quality,
        aspectRatio: settings.value.aspectRatio,
        durationSec: (scene.targetDurationMs / 1000) as 5 | 10 | 15,
        prompt: scene.visualPlan,
        narration: scene.narration,
        seed: crypto.getRandomValues(new Uint32Array(1))[0],
        assetIds: scene.assetIds,
        discardH3Audio: settings.value.discardH3Audio,
      });
      await save(scene.id, {
        lastJobId: submitted.id,
        pendingRequestId: undefined,
        status: sceneStatusFor(submitted),
        generationStage: submitted.stageMessage,
      });
      submittedCount += 1;
      void refreshTask(scene.id, submitted.id);
    } catch (error) {
      const message = normalizeConnectionFailure(error).message;
      failures.push(`${scene.title}：${message}`);
      await save(scene.id, { status: "failed", generationStage: message });
    }
  }
  batchSubmitting.value = false;
  batchNotice.value = failures.length
    ? `已排入 ${submittedCount} 个，${failures.length} 个未提交：${failures[0]}`
    : `${submittedCount} 个分镜已排入队列，可在右上角“任务”中查看。`;
}

async function openFullPreview() {
  const scene = scenes.value[0];
  if (!scene || previewBusy.value) return;
  previewBusy.value = true;
  taskError.value = "";
  try {
    const result = await requestNativePreview(scene.projectId, activeFrameProfile.value.aspectRatio);
    fullPreviewUrl.value = convertFileSrc(result.outputPath);
    fullPreviewOpen.value = true;
  } catch (error) {
    taskError.value = normalizeConnectionFailure(error).message;
  } finally {
    previewBusy.value = false;
  }
}

async function makeEnhancement() {
  const scene = selectedScene.value;
  const source = candidateVersions.value.find((item) => item.id === scene?.selectedVersionId);
  if (!scene || !source || enhancing.value) return;
  taskError.value = "";
  if (settings.value.aspectRatio !== "16:9") {
    taskError.value = "当前已验证的 SeedVR2 增强流程仅支持 16:9；其他画幅验证完成后再开放。";
    return;
  }
  enhancing.value = true;
  taskKind.value = "enhancement";
  const requestId = scene.pendingRequestId ?? crypto.randomUUID();
  await save(scene.id, {
    pendingRequestId: requestId,
    status: "generating",
    generationStage: "正在准备 SeedVR2 1080p 任务",
  });
  try {
    const submitted = await provider.submitUpscale({
      clientRequestId: requestId,
      projectId: scene.projectId,
      sceneId: scene.id,
      sourcePath: source.localPath,
      seed: crypto.getRandomValues(new Uint32Array(1))[0],
    });
    task.value = submitted;
    await save(scene.id, {
      lastUpscaleJobId: submitted.id,
      pendingRequestId: undefined,
      status: "generating",
      generationStage: "正在制作 1080p 增强版",
    });
    await refreshEnhancementTask(scene.id, submitted.id, source.id);
  } catch (error) {
    taskError.value = normalizeConnectionFailure(error).message;
    await save(scene.id, { status: "approved", generationStage: taskError.value });
  } finally {
    enhancing.value = false;
  }
}

async function cancelTask() {
  const scene = selectedScene.value;
  if (!scene || !task.value) return;
  const jobId = taskKind.value === "enhancement" ? scene.lastUpscaleJobId : scene.lastJobId;
  if (!jobId) return;
  try {
    await provider.cancel(jobId);
    if (taskKind.value === "enhancement" && scene.selectedVersionId) {
      await refreshEnhancementTask(scene.id, jobId, scene.selectedVersionId);
    } else {
      await refreshTask(scene.id, jobId);
    }
  } catch (error) {
    taskError.value = normalizeConnectionFailure(error).message;
  }
}

async function recoverPendingSceneJob(sceneId: string): Promise<boolean> {
  const scene = scenes.value.find((item) => item.id === sceneId);
  if (!scene?.pendingRequestId || scene.lastJobId || scene.lastUpscaleJobId) return false;
  try {
    const jobs = await serviceRepository.listLocalJobs(scene.projectId);
    const queued = jobs?.find((item) => item.clientRequestId === scene.pendingRequestId);
    if (!queued) return false;
    if (queued.status === "submit_failed") {
      taskError.value = queued.errorMessage ?? "任务提交失败，可重新生成并安全复用原请求。";
      update(scene.id, { status: "failed", generationStage: taskError.value });
      return true;
    }
    if (!queued.remoteJobId) return false;
    const enhancement = queued.kind === "video_upscale";
    await save(scene.id, {
      pendingRequestId: undefined,
      lastJobId: enhancement ? scene.lastJobId : queued.remoteJobId,
      lastUpscaleJobId: enhancement ? queued.remoteJobId : scene.lastUpscaleJobId,
      status: "generating",
      generationStage: "已从本地任务队列恢复远端任务",
    });
    if (enhancement && scene.selectedVersionId) {
      await refreshEnhancementTask(scene.id, queued.remoteJobId, scene.selectedVersionId);
    } else {
      await refreshTask(scene.id, queued.remoteJobId);
    }
    return true;
  } catch (error) {
    taskError.value = normalizeConnectionFailure(error).message;
    return false;
  }
}

watch(selectedSceneId, (sceneId) => {
  task.value = undefined;
  taskError.value = "";
  const scene = scenes.value.find((item) => item.id === sceneId);
  void loadNarration(scene?.projectId, scene?.id);
  void loadReferenceAssets(scene?.projectId);
  void loadCandidateVersions(scene?.projectId, scene?.id).then(async () => {
    if (!scene) return;
    if (await recoverPendingSceneJob(scene.id)) return;
    if (scene.lastUpscaleJobId && scene.selectedVersionId) {
      await refreshEnhancementTask(scene.id, scene.lastUpscaleJobId, scene.selectedVersionId);
    } else if (scene.lastJobId) {
      await refreshTask(scene.id, scene.lastJobId);
    }
  });
}, { immediate: true });

watch([loading, () => scenes.value[0]?.projectId], async ([isLoading, projectId]) => {
  if (isLoading || !projectId || recoveredProjectId === projectId) return;
  recoveredProjectId = projectId;
  for (const scene of scenes.value) {
    if (await recoverPendingSceneJob(scene.id)) continue;
    if (scene.lastUpscaleJobId && scene.selectedVersionId) {
      void refreshEnhancementTask(scene.id, scene.lastUpscaleJobId, scene.selectedVersionId);
    } else if (scene.lastJobId) {
      void refreshTask(scene.id, scene.lastJobId);
    }
  }
}, { immediate: true });

onBeforeUnmount(() => { clearPoll(); narrationAudio?.pause(); });
const confirmRemove = () => {
  if (scenes.value.length <= 1) return;
  if (window.confirm(`删除分镜“${selectedScene.value?.title}”？此操作只删除当前本地草稿。`)) removeSelected();
};
</script>

<template>
  <section class="page storyboard-page">
    <header class="story-head">
      <div><p class="breadcrumb">为什么会打雷？　/　分镜</p><div class="page-title-line"><h1>分镜编排</h1><span class="status-line"><span class="dot gray"></span><strong>优云智算</strong><b>无卡模式</b><span>|</span><span>提交生成时自动切换 GPU</span></span></div></div>
      <div class="head-actions"><div class="format-pill"><span>项目画幅</span><select :value="settings.aspectRatio" aria-label="项目画幅" @change="setProjectAspect"><option value="16:9">16:9</option><option value="9:16">9:16</option><option value="4:3">4:3</option><option value="3:4">3:4</option><option value="1:1">1:1</option></select><i></i><span>候选画面</span><b>{{ activeFrameProfile.visibleWidth }}×{{ activeFrameProfile.visibleHeight }}</b></div><button class="btn primary" :disabled="submitting || batchSubmitting || enhancing || isTaskActive || !selectedScene" @click="generateSelected"><LoaderCircle v-if="submitting" class="spin" :size="19"/><Sparkles v-else :size="19"/>{{ submitting ? '正在提交' : '生成选中分镜' }}</button><button class="btn" :disabled="batchSubmitting || submitting || enhancing || !scenes.length" :title="batchNotice || '将尚无候选版本的分镜依次排入队列'" @click="generateIncompleteScenes"><LoaderCircle v-if="batchSubmitting" class="spin" :size="18"/><Film v-else :size="18"/>{{ batchSubmitting ? '正在排队' : '生成未完成分镜' }}</button><button class="btn" :disabled="previewBusy || !scenes.length" @click="openFullPreview"><LoaderCircle v-if="previewBusy" class="spin" :size="18"/><Play v-else :size="18"/>{{ previewBusy ? '正在合成' : '预览全片' }}</button></div>
    </header>

    <div class="story-workspace">
      <aside class="panel shot-panel">
        <div class="panel-head"><h2>{{ scenes.length }} 个分镜</h2><div class="shot-actions"><button title="上移" @click="moveSelected(-1)"><ArrowUp :size="16"/></button><button title="下移" @click="moveSelected(1)"><ArrowDown :size="16"/></button><button title="复制" @click="duplicateSelected"><Copy :size="16"/></button><button title="删除" @click="confirmRemove"><Trash2 :size="16"/></button><button class="add" title="新增分镜" @click="add"><Plus :size="18"/></button></div></div>
        <div v-if="loading" class="empty-storyboard"><LoaderCircle class="spin" :size="25"/><b>正在读取项目分镜</b></div>
        <div v-else-if="!scenes.length" class="empty-storyboard"><Film :size="30"/><b>{{ loadError || '当前项目还没有分镜' }}</b><span>新增一条分镜，填写画面与旁白后即可提交生成。</span><button class="btn primary" :disabled="Boolean(loadError && loadError.includes('先'))" @click="add"><Plus :size="16"/>新增分镜</button></div>
        <div v-else class="shot-list">
          <article v-for="(shot,index) in scenes" :key="shot.id" :class="{selected:selectedSceneId===shot.id}" @click="select(shot.id)">
            <span class="grab">⠿</span><div class="shot-thumb lightning-image"><b>{{ String(index + 1).padStart(2, '0') }}</b></div><div class="shot-copy"><strong>{{ shot.title }}</strong><p>{{ shot.narration || shot.purpose || '填写旁白与画面描述' }}</p><span :class="`${statusTone[shot.status]}-text`"><i class="dot" :class="statusTone[shot.status]"></i>{{ statusLabel[shot.status] }}</span></div><time>{{ shot.targetDurationMs / 1000 }}秒</time>
          </article>
        </div>
      </aside>

      <section class="preview-column">
        <div class="video-card panel">
          <div class="video-preview" :class="{ 'lightning-image': !previewMedia }">
            <video v-if="previewMedia" :key="previewMedia.id" :src="previewMedia.previewUrl" controls preload="metadata"></video>
            <span class="video-label">{{ settings.aspectRatio }} · {{ previewEnhanced ? '1080p 增强版' : previewVersion ? previewVersion.workflowId : '候选预览' }}</span>
            <span v-if="!previewMedia" class="timecode">等待生成</span>
            <div v-if="!previewMedia" class="caption">{{ selectedScene?.narration || '生成完成后在这里预览候选视频。' }}</div>
          </div>
          <div v-if="!previewVersion" class="player"><button><Play :size="19" fill="currentColor"/></button><button>Ⅰ◀</button><button>▶Ⅰ</button><strong>00:00</strong><span>/ --:--</span><div class="scrub"><i style="width:0"></i></div><Volume2 :size="19"/><div class="volume"><i style="width:55%"></i></div><Maximize2 :size="18"/></div>
        </div>
        <div class="version-row">
          <div class="version-strip">
            <button v-for="(version,index) in candidateVersions" :key="version.id" class="version" :class="{ active: !previewEnhanced && previewVersion?.id === version.id }" @click="previewVersionId=version.id; previewEnhancedId='' "><b>V{{ index + 1 }}</b><span>{{ version.selected ? '正式版本' : '候选版本' }}</span></button>
            <button v-if="enhancedForOfficial" class="version final-version" :class="{ active: previewEnhanced?.id === enhancedForOfficial.id }" @click="previewEnhancedId=enhancedForOfficial.id"><b>1080p</b><span>增强版已就绪</span></button>
            <button v-if="!candidateVersions.length" class="version" disabled><b>V1</b><span>等待候选</span></button>
          </div>
          <button class="btn" :disabled="submitting || enhancing || isTaskActive" @click="generateSelected"><Plus :size="16"/>重新生成候选</button>
          <button class="btn" :disabled="!canSelectOfficial" @click="selectOfficialVersion"><Star :size="16"/>设为正式版本</button>
          <button class="btn primary" :disabled="!canMakeEnhancement" :title="settings.aspectRatio === '16:9' ? '为正式版本制作可选的 1920×1080 AI 增强文件' : '当前仅开放已验证的 16:9 SeedVR2 增强流程'" @click="makeEnhancement"><LoaderCircle v-if="enhancing" class="spin" :size="16"/>{{ enhancedForOfficial ? '重新制作 1080p 增强版' : enhancing ? '正在提交' : '制作 1080p 增强版' }}</button>
        </div>
      </section>

      <aside class="panel inspector">
        <div class="inspector-tabs"><button>内容</button><button>画面</button><button class="active">生成</button></div>
        <div class="inspector-body">
          <label class="section-label">生成方式</label>
          <div class="mode-grid"><button v-for="item in modes" :key="item.id" :class="{active:selectedScene?.generationMode===item.id}" @click="chooseMode(item.id)"><span>{{ item.symbol }}</span>{{ item.label }}</button></div>
          <div v-if="selectedScene && selectedScene.generationMode !== 't2v'" class="reference-inputs">
            <div class="field-head"><label class="section-label">参考素材</label><RouterLink to="/assets">管理素材　›</RouterLink></div>
            <label v-if="selectedScene.generationMode === 'i2v' || selectedScene.generationMode === 'continue'"><span>首帧图片</span><select :value="selectedReferenceIds[0] ?? ''" @change="setReferenceSlot(0, 'image', $event)"><option value="">请选择图片</option><option v-for="asset in imageAssets" :key="asset.id" :value="asset.id">{{ asset.name }}</option></select></label>
            <template v-else-if="selectedScene.generationMode === 'flf2v'">
              <label><span>首帧图片</span><select :value="selectedReferenceIds[0] ?? ''" @change="setReferenceSlot(0, 'image', $event)"><option value="">请选择图片</option><option v-for="asset in imageAssets" :key="asset.id" :value="asset.id">{{ asset.name }}</option></select></label>
              <label><span>尾帧图片</span><select :value="selectedReferenceIds[1] ?? ''" :disabled="!selectedReferenceIds[0]" @change="setReferenceSlot(1, 'image', $event)"><option value="">请选择图片</option><option v-for="asset in imageAssets" :key="asset.id" :value="asset.id">{{ asset.name }}</option></select></label>
            </template>
            <template v-else-if="selectedScene.generationMode === 'r2v'">
              <label><span>参考视频</span><select :value="selectedReferenceIds[0] ?? ''" @change="setReferenceSlot(0, 'video', $event)"><option value="">请选择视频</option><option v-for="asset in videoAssets" :key="asset.id" :value="asset.id">{{ asset.name }}</option></select></label>
              <label><span>参考图片</span><select :value="selectedReferenceIds[1] ?? ''" :disabled="!selectedReferenceIds[0]" @change="setReferenceSlot(1, 'image', $event)"><option value="">请选择图片</option><option v-for="asset in imageAssets" :key="asset.id" :value="asset.id">{{ asset.name }}</option></select></label>
            </template>
            <p v-if="!referenceAssets.length">当前项目暂无可用素材，请先到素材页导入。</p>
          </div>
          <div class="field-head"><label class="section-label">系统旁白 / 解说文案</label><a>♟ 重写文案</a></div>
          <textarea :value="selectedScene?.narration" @input="updateNarrationText"></textarea>
          <div class="field-head"><label class="section-label">时长</label><label class="section-label">配音音色</label></div>
          <div class="voice-row"><div class="segmented"><button v-for="seconds in [5,10,15] as const" :key="seconds" :class="{active:durationLabel===`${seconds}秒`}" @click="setDuration(seconds)">{{ seconds }}秒</button></div><select v-model="selectedVoiceId" class="select" aria-label="系统旁白音色"><option v-for="voice in systemVoices" :key="voice.id" :value="voice.id">{{ voice.name }} · {{ voice.locale }}</option></select><button class="round" :disabled="narrationBusy" @click="playNarration"><LoaderCircle v-if="narrationBusy" class="spin" :size="15"/><Play v-else :size="15" fill="currentColor"/></button><button class="btn compact" :disabled="narrationBusy" @click="synthesizeNarration(false)"><RotateCcw :size="15"/>{{ narrationArtifact ? '重新生成' : '生成旁白' }}</button></div>
          <div class="narration-import-row"><select v-model="selectedNarrationAssetId" class="select" aria-label="导入旁白录音"><option value="">从素材库选择已有录音</option><option v-for="asset in audioAssets" :key="asset.id" :value="asset.id">{{ asset.name }}</option></select><button class="btn compact" :disabled="narrationBusy || !selectedNarrationAssetId" @click="importNarrationAudio"><Music2 :size="15"/>使用录音</button><RouterLink to="/assets">管理音频素材　›</RouterLink></div>
          <p v-if="narrationError" class="task-error narration-error">{{ narrationError }}</p>
          <div class="field-head"><label class="section-label">生成设置</label><span class="warning-text">预计生成约 3 分钟</span></div>
          <div class="quality-grid"><button :class="{active:selectedScene?.quality==='fast'}" @click="setQuality('fast')"><b>快速 8 步</b><span>约 3 分钟，适合快速预览</span></button><button :class="{active:selectedScene?.quality==='high'}" @click="setQuality('high')"><b>高质量 20 步</b><span>约 5 分钟，会重新生成内容</span></button></div>
          <label class="section-label audio-label">音轨设置</label><div class="audio-setting"><span class="round filled"><Pause :size="13" fill="currentColor"/></span><div><b>不使用 H3 原生音轨</b><span>{{ narrationArtifact ? `系统旁白已生成 · ${(narrationArtifact.durationMs / 1000).toFixed(1)} 秒` : '正式旁白使用系统 TTS' }}</span></div><a>修改</a></div>
          <div class="task-card"><div class="field-head"><h3>当前任务 · {{ taskKind === 'enhancement' ? '1080p 增强版' : '候选生成' }}</h3><button v-if="task && !['completed','failed','cancelled','interrupted'].includes(task.status)" class="task-cancel" type="button" @click="cancelTask"><X :size="14"/>取消任务</button></div><div class="task-main"><div class="task-thumb lightning-image"></div><div><b>{{ task ? taskStatusLabel : taskError ? '任务未提交' : '当前没有生成任务' }}</b><div class="progress"><i :style="{ width: `${taskPercent}%` }"></i></div><span :class="{ 'task-error': taskError }">{{ taskDescription }}</span></div><strong>{{ task ? `${taskPercent}%` : taskError ? '需处理' : '空闲' }}</strong></div><footer><span class="dot" :class="{ gray: !task || ['completed','failed','cancelled','interrupted'].includes(task.status) }"></span><b>知画服务队列</b><span>|　{{ task?.id ? `任务 ${task.id.slice(0, 8)}` : '提交前不会启动 GPU' }}</span></footer></div>
        </div>
      </aside>
    </div>

    <section class="panel timeline">
      <div class="timeline-toolbar"><button><Play :size="20" fill="currentColor"/></button><b>00:18.4</b><span>/ 00:42.0</span><span class="shutdown"><span class="dot"></span>队列结束后自动关机</span><span class="zoom">−　━━━━　＋　 <button>适应</button></span></div>
      <div class="ruler"><span>0:00</span><span>0:10</span><span>0:20</span><span>0:30</span><span>0:40</span></div>
      <div class="tracks">
        <div class="track-labels"><span><Film :size="18"/>画面</span><span>♩　旁白</span><span><Subtitles :size="18"/>字幕</span><span><Music2 :size="18"/>音乐</span></div>
        <div class="track-content"><div class="video-track"><i v-for="(shot,index) in scenes" :key="shot.id" class="lightning-image"><b>{{ String(index + 1).padStart(2, '0') }}</b> {{ shot.title }}</i></div><div class="voice-track"><i v-for="shot in scenes" :key="shot.id">▥　{{ shot.title }}</i></div><div class="subtitle-track"><i v-for="shot in scenes" :key="shot.id">{{ shot.title }}</i></div><div class="music-track">♫　轻柔科普氛围音乐 -18 dB　﹏﹏﹏﹏﹏﹏﹏﹏﹏﹏﹏﹏﹏﹏</div><div class="playhead"></div></div>
      </div>
    </section>

    <div v-if="fullPreviewOpen" class="full-preview-backdrop" @click.self="fullPreviewOpen=false">
      <section class="full-preview-dialog">
        <header><div><h2>全片预览</h2><p>使用正式候选、系统旁白和字幕在本机临时合成</p></div><button type="button" aria-label="关闭全片预览" @click="fullPreviewOpen=false"><X :size="21"/></button></header>
        <video :src="fullPreviewUrl" controls autoplay></video>
      </section>
    </div>
  </section>
</template>

<style scoped>
.storyboard-page{display:grid;grid-template-rows:90px minmax(0,1fr) 280px;gap:12px}.story-head{display:flex;align-items:center;justify-content:space-between;padding:0 4px}.breadcrumb{color:#54698e;margin-bottom:7px}.format-pill{height:49px;padding:0 14px;border:1px solid var(--line);border-radius:9px;background:#fff;display:flex;align-items:center;gap:8px;font-size:13px}.format-pill b{font-size:15px}.format-pill select{border:1px solid #ccd9ea;border-radius:6px;background:#f7faff;color:#12335f;font-weight:750;padding:5px 24px 5px 8px}.format-pill i{height:20px;width:1px;background:#dce4ef}.story-workspace{min-height:0;display:grid;grid-template-columns:345px minmax(500px,1fr) 405px;gap:14px}.shot-panel{overflow:hidden}.shot-panel .panel-head{height:50px}.shot-list{padding:0 8px}.shot-list article{height:99px;display:grid;grid-template-columns:20px 98px 1fr 39px;gap:8px;align-items:center;border-bottom:1px solid #e5ebf4;padding:8px 3px;border-radius:8px}.shot-list article.selected{border:2px solid var(--blue);background:#f1f6ff}.grab{color:#6a7c9a;font-size:20px}.shot-thumb{height:80px;border-radius:6px;position:relative}.shot-thumb b{position:absolute;top:5px;left:5px;color:#fff;background:rgba(7,19,38,.78);padding:3px 5px;border-radius:4px}.shot-copy{min-width:0}.shot-copy>strong{font-size:16px}.shot-copy p{margin:5px 0;font-size:12px;color:#617697;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.shot-copy span{font-size:12px;display:flex;align-items:center;gap:6px}.shot-copy .dot{width:8px;height:8px}.shot-copy .dot.primary{background:var(--blue)}.shot-copy .dot.warning{background:var(--orange)}.shot-copy .dot.muted{background:#9aaac1}.shot-list time{align-self:end;padding-bottom:8px;color:#657794;font-size:12px}.preview-column{min-width:0;display:flex;flex-direction:column;gap:12px}.video-card{flex:1;min-height:0;overflow:hidden;display:flex;flex-direction:column}.video-preview{flex:1;position:relative;min-height:280px;background:#0a1728}.video-preview video{width:100%;height:100%;object-fit:contain;background:#060d18}.video-label,.timecode{position:absolute;z-index:2;top:9px;color:#fff;background:rgba(5,16,35,.78);border-radius:5px;padding:6px 9px;font-size:12px}.video-label{left:9px}.timecode{right:9px}.caption{position:absolute;bottom:14px;left:50%;transform:translateX(-50%);white-space:nowrap;background:rgba(3,14,29,.79);color:#fff;border-radius:5px;padding:7px 16px;font-size:19px;font-weight:650}.player{height:54px;display:flex;align-items:center;gap:13px;padding:0 16px}.player button{border:0;background:transparent;color:#142d57}.player span{color:#7082a0}.scrub,.volume{height:7px;border-radius:9px;background:#dce5f0;position:relative}.scrub{flex:1}.volume{width:62px}.scrub i,.volume i{position:absolute;height:100%;left:0;border-radius:inherit;background:var(--blue)}.version-row{height:58px;display:flex;gap:8px}.version-strip{display:flex;gap:6px;max-width:276px;overflow-x:auto}.version{width:86px;flex:0 0 86px;border:1px solid #d5e1ef;border-radius:9px;background:#fff;display:flex;flex-direction:column;justify-content:center;align-items:flex-start;padding-left:13px}.version b{font-size:16px}.version span{font-size:11px;color:#677b9c}.version.active{border-color:var(--blue);background:#edf5ff;color:var(--blue)}.version.final-version{border-color:#8bd6b5;background:#f1fbf7;color:#11845d}.version-row>.btn{flex:1;padding:0 10px}.inspector{overflow:hidden}.inspector-tabs{height:44px;border-bottom:1px solid var(--line);display:grid;grid-template-columns:repeat(3,1fr)}.inspector-tabs button{border:0;background:transparent;font-weight:700;font-size:15px;position:relative}.inspector-tabs .active{color:var(--blue)}.inspector-tabs .active:after{content:"";position:absolute;left:22%;right:22%;bottom:0;height:3px;background:var(--blue)}.inspector-body{height:calc(100% - 44px);overflow-y:auto;padding:13px 16px}.mode-grid{display:grid;grid-template-columns:repeat(4,1fr);gap:7px;margin:9px 0 15px}.mode-grid button{height:65px;border:1px solid #d9e3f0;border-radius:7px;background:#fff;font-size:11px;color:#334b71}.mode-grid button span{display:block;font-size:19px;margin-bottom:5px}.mode-grid button.active{border-color:var(--blue);background:#edf5ff;color:var(--blue)}.field-head{display:flex;align-items:center;justify-content:space-between;margin:10px 0 7px}.field-head a{font-size:12px;color:var(--blue)}textarea{width:100%;height:73px;border:1px solid #d4dfef;border-radius:7px;resize:none;padding:11px;font-size:13px;line-height:1.6;background:#fff}.voice-row{display:flex;gap:7px}.segmented{gap:0;background:#eef3fa;border-radius:7px;padding:3px}.segmented button{height:35px;border:0;border-radius:6px;background:transparent;color:#53698d;font-size:12px}.segmented .active{background:#fff;color:var(--blue);font-weight:700}.select{flex:1;border:1px solid #d4dfef;border-radius:7px;background:#fff;display:flex;align-items:center;justify-content:center;gap:5px;font-size:12px}.round{width:38px;height:38px;border:1px solid #cfe0f5;border-radius:50%;display:grid;place-items:center;background:#fff;color:var(--blue)}.compact{min-height:38px;padding:0 8px;font-size:12px}.quality-grid{display:grid;grid-template-columns:1fr 1fr;gap:8px}.quality-grid button{height:64px;border:1px solid #d8e2ef;border-radius:8px;background:#fff;text-align:left;padding:9px 12px}.quality-grid b,.quality-grid span{display:block}.quality-grid span{font-size:11px;color:#6c809f;margin-top:5px}.quality-grid .active{border-color:var(--blue);background:#eff6ff;color:var(--blue)}.audio-label{display:block;margin:13px 0 7px}.audio-setting{height:61px;border:1px solid #d8e2ef;border-radius:8px;display:flex;align-items:center;gap:10px;padding:8px 12px}.round.filled{width:27px;height:27px;background:var(--blue);color:#fff}.audio-setting>div{display:flex;flex-direction:column}.audio-setting span{font-size:11px;color:#7183a0;margin-top:3px}.audio-setting a{margin-left:auto;color:var(--blue);font-size:12px}.task-card{margin-top:16px;border:1px solid #dce5f0;border-radius:9px;padding:10px}.task-main{display:grid;grid-template-columns:72px 1fr 42px;gap:10px;align-items:center}.task-thumb{height:58px;border-radius:7px}.task-main b{font-size:12px}.task-main .progress{margin:7px 0}.task-main span{font-size:10px;color:#6c809d}.task-main>strong{color:#f25b20}.task-card footer{display:flex;gap:7px;align-items:center;border-top:1px solid #e8edf4;margin-top:9px;padding-top:8px;font-size:11px}.timeline{overflow:hidden}.timeline-toolbar{height:47px;border-bottom:1px solid var(--line);display:flex;align-items:center;gap:10px;padding:0 18px}.timeline-toolbar>button{width:32px;height:32px;border:0;border-radius:50%;display:grid;place-items:center;background:var(--blue);color:#fff}.shutdown{margin-left:auto;margin-right:auto;background:#e6f8ef;color:#158b60;padding:6px 12px;border-radius:7px;font-size:12px}.shutdown .dot{display:inline-block;width:8px;height:8px;margin-right:7px}.zoom{margin-left:auto}.zoom button{height:32px;border:1px solid #d6e1ef;background:#fff;border-radius:7px}.ruler{height:27px;margin-left:113px;display:flex;justify-content:space-between;align-items:end;padding:0 12px 3px;font-size:11px;color:#607598;border-bottom:1px solid #dfe7f2}.tracks{height:202px;display:grid;grid-template-columns:113px 1fr}.track-labels{display:grid;grid-template-rows:repeat(4,1fr);border-right:1px solid var(--line)}.track-labels span{display:flex;align-items:center;gap:9px;padding-left:15px;border-bottom:1px solid #e5ebf3;font-weight:700;font-size:13px}.track-content{position:relative;display:grid;grid-template-rows:repeat(4,1fr);padding:6px 8px}.video-track,.voice-track,.subtitle-track{display:flex;gap:4px;min-width:0}.video-track i,.voice-track i,.subtitle-track i{font-style:normal;flex:1;min-width:0;border-radius:6px;padding:8px;color:#fff;font-size:11px;white-space:nowrap;overflow:hidden}.voice-track i{background:#ddf4ea;color:#27886b;text-align:center}.subtitle-track i{background:#ede6ff;color:#4d45c9;text-align:center}.music-track{margin-top:3px;border-radius:6px;background:#fff1d9;color:#dc7116;padding:9px;font-size:11px;white-space:nowrap;overflow:hidden}.playhead{position:absolute;left:42%;top:0;bottom:0;width:2px;background:#ff384f}.playhead:before{content:"";position:absolute;top:-1px;left:-4px;border-left:5px solid transparent;border-right:5px solid transparent;border-top:8px solid #ff384f}@media(max-width:1380px){.story-workspace{grid-template-columns:310px minmax(450px,1fr) 370px}.storyboard-page{grid-template-rows:84px minmax(0,1fr) 250px}.timeline{height:250px}.version-row .btn{font-size:12px}.format-pill{display:none}.shot-list article{grid-template-columns:17px 88px 1fr 34px}.shot-thumb{height:72px}.caption{font-size:16px}}
.shot-actions{display:flex;align-items:center;gap:3px}.shot-actions button{width:28px;height:28px;padding:0;border:0;border-radius:6px;display:grid;place-items:center;color:#506787;background:transparent}.shot-actions button:hover{color:var(--blue);background:#edf4ff}.shot-actions .add{width:34px;height:34px;margin-left:3px;border:1px solid #cfe0f3;background:#fff}
.head-actions .btn:disabled,.version-row .btn:disabled,.version:disabled{opacity:.58;cursor:not-allowed}.spin{animation:spin .9s linear infinite}.task-cancel{border:0;background:transparent;color:#d6463c;display:flex;align-items:center;gap:4px;font-size:12px}.task-main .task-error{color:#c53d35}.task-main .progress i{display:block;height:100%;border-radius:inherit;background:var(--blue);transition:width .25s ease}@keyframes spin{to{transform:rotate(360deg)}}
.empty-storyboard{height:calc(100% - 50px);padding:28px;display:flex;flex-direction:column;align-items:center;justify-content:center;text-align:center;gap:10px;color:#617697}.empty-storyboard b{color:#29466f}.empty-storyboard span{max-width:245px;font-size:12px;line-height:1.6}.empty-storyboard .btn{margin-top:7px}
.reference-inputs{margin:-3px 0 12px;padding:9px 10px;border:1px solid #d9e4f1;border-radius:8px;background:#f8fbff;display:grid;grid-template-columns:1fr 1fr;gap:7px}.reference-inputs .field-head{grid-column:1/-1;margin:0 0 2px}.reference-inputs .field-head a{color:var(--blue);font-size:12px}.reference-inputs label{display:flex;flex-direction:column;gap:4px;color:#52698c;font-size:11px}.reference-inputs select{height:34px;min-width:0;border:1px solid #cfdced;border-radius:6px;background:#fff;padding:0 8px;color:#203b65}.reference-inputs p{grid-column:1/-1;color:#7385a2;font-size:11px}
.narration-import-row{display:grid;grid-template-columns:1fr auto auto;gap:7px;align-items:center;margin-top:7px}.narration-import-row .select{height:34px}.narration-import-row a{color:var(--blue);font-size:11px;white-space:nowrap}
.full-preview-backdrop{position:fixed;z-index:95;inset:30px 0 0;display:grid;place-items:center;background:rgba(5,18,40,.72);backdrop-filter:blur(3px)}.full-preview-dialog{width:min(1050px,82vw);overflow:hidden;border:1px solid #6d7f9b;border-radius:13px;background:#071326;box-shadow:0 26px 80px rgba(0,0,0,.38)}.full-preview-dialog header{height:68px;padding:0 18px;display:flex;align-items:center;justify-content:space-between;color:#fff;background:#0d1c33}.full-preview-dialog header h2{color:#fff}.full-preview-dialog header p{margin-top:4px;color:#aab9cf;font-size:12px}.full-preview-dialog header button{width:38px;height:38px;border:0;border-radius:8px;display:grid;place-items:center;color:#d7e2f1;background:transparent}.full-preview-dialog header button:hover{background:#1a2d49}.full-preview-dialog video{display:block;width:100%;max-height:calc(82vh - 98px);aspect-ratio:16/9;object-fit:contain;background:#000}
</style>
