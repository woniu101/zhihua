<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import {
  BellRing,
  CheckCircle2,
  CircleAlert,
  Film,
  ImagePlus,
  LoaderCircle,
  RefreshCw,
  Sparkles,
  WandSparkles,
  X,
} from "lucide-vue-next";
import type { ZhihuaProject } from "../domain/projects";
import { projectRepository } from "../services/projectRepository";
import {
  normalizeConnectionFailure,
  serviceRepository,
  type LocalGenerationJob,
} from "../services/serviceRepository";

const router = useRouter();
const open = ref(false);
const loading = ref(false);
const jobs = ref<LocalGenerationJob[]>([]);
const projects = ref<ZhihuaProject[]>([]);
const refreshError = ref("");
const cancellingId = ref("");
let timer: ReturnType<typeof setInterval> | undefined;

const activeStatuses = new Set([
  "pending_submit",
  "leased",
  "queued",
  "waiting_for_compute",
  "preparing",
  "uploading",
  "running",
  "upscaling",
  "downloading",
]);
const visibleJobs = computed(() => [...jobs.value]
  .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
  .slice(0, 24));
const attentionCount = computed(() => jobs.value.filter((job) =>
  activeStatuses.has(job.status) || job.status === "completed"
).length);

function projectName(job: LocalGenerationJob): string {
  return projects.value.find((item) => item.id === job.projectId)?.title ?? "本地项目";
}

function taskName(job: LocalGenerationJob): string {
  return {
    image_generation: "图片生成",
    image_edit: "图片编辑",
    video_candidate: "候选视频",
    video_upscale: "1080p 增强",
  }[job.kind] ?? "生成任务";
}

function taskIcon(job: LocalGenerationJob) {
  if (job.kind === "image_generation") return ImagePlus;
  if (job.kind === "image_edit") return WandSparkles;
  return Film;
}

function statusMeta(status: string): { label: string; tone: string; icon: typeof LoaderCircle } {
  if (status === "completed_local") return { label: "已保存", tone: "success", icon: CheckCircle2 };
  if (status === "completed") return { label: "等待保存", tone: "ready", icon: BellRing };
  if (status === "failed") return { label: "失败", tone: "danger", icon: CircleAlert };
  if (status === "cancelled" || status === "interrupted") return { label: "已取消", tone: "muted", icon: X };
  if (status === "queued" || status === "waiting_for_compute" || status === "pending_submit" || status === "leased") {
    return { label: "排队中", tone: "queued", icon: LoaderCircle };
  }
  if (status === "upscaling") return { label: "增强中", tone: "running", icon: Sparkles };
  if (status === "downloading") return { label: "保存中", tone: "running", icon: LoaderCircle };
  return { label: "生成中", tone: "running", icon: LoaderCircle };
}

function timeLabel(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

async function refresh(syncRemote = true): Promise<void> {
  if (loading.value) return;
  loading.value = true;
  refreshError.value = "";
  try {
    const [localJobs, projectList] = await Promise.all([
      serviceRepository.listLocalJobs(),
      projects.value.length ? Promise.resolve(projects.value) : projectRepository.list(),
    ]);
    jobs.value = localJobs ?? [];
    projects.value = projectList;
    if (syncRemote) {
      const remoteActive = jobs.value.filter((job) =>
        job.remoteJobId && activeStatuses.has(job.status)
      );
      if (remoteActive.length) {
        await Promise.allSettled(remoteActive.map((job) => serviceRepository.syncJob(job.remoteJobId!)));
        jobs.value = await serviceRepository.listLocalJobs() ?? jobs.value;
      }
    }
  } catch (error) {
    refreshError.value = normalizeConnectionFailure(error).message;
  } finally {
    loading.value = false;
  }
}

async function toggle(): Promise<void> {
  open.value = !open.value;
  if (open.value) await refresh();
}

async function visit(job: LocalGenerationJob): Promise<void> {
  await projectRepository.markOpened(job.projectId);
  open.value = false;
  await router.push(job.kind.startsWith("image_") ? "/assets" : "/storyboard");
}

async function cancel(job: LocalGenerationJob): Promise<void> {
  if (!job.remoteJobId || cancellingId.value) return;
  cancellingId.value = job.remoteJobId;
  refreshError.value = "";
  try {
    await serviceRepository.cancelJob(job.remoteJobId);
    await refresh(false);
  } catch (error) {
    refreshError.value = normalizeConnectionFailure(error).message;
  } finally {
    cancellingId.value = "";
  }
}

onMounted(() => {
  void refresh();
  timer = setInterval(() => void refresh(open.value), 6_000);
});
onBeforeUnmount(() => {
  if (timer) clearInterval(timer);
});
</script>

<template>
  <button class="titlebar-task-button" type="button" :class="{ active: open }" @click="toggle">
    <LoaderCircle v-if="loading" :size="14" class="spin" />
    <BellRing v-else :size="15" />
    <span>任务</span>
    <b v-if="attentionCount">{{ attentionCount > 99 ? "99+" : attentionCount }}</b>
  </button>

  <Teleport to="body">
    <div v-if="open" class="task-center-backdrop" @click.self="open = false">
      <aside class="task-center" aria-label="生成任务中心">
        <header>
          <div><h2>生成任务</h2><p>图片、视频与高清增强进度</p></div>
          <div class="task-center-head-actions">
            <button type="button" aria-label="刷新任务" title="刷新" @click="refresh()"><RefreshCw :size="18" :class="{ spin: loading }" /></button>
            <button type="button" aria-label="关闭任务中心" title="关闭" @click="open = false"><X :size="20" /></button>
          </div>
        </header>

        <p v-if="refreshError" class="task-center-error">{{ refreshError }}</p>
        <div v-if="!visibleJobs.length && !loading" class="task-center-empty">
          <CheckCircle2 :size="34" />
          <b>当前没有生成任务</b>
          <span>创建图片或候选视频后，进度会显示在这里。</span>
        </div>

        <div v-else class="task-center-list">
          <article v-for="job in visibleJobs" :key="job.clientRequestId" class="task-item">
            <div class="task-item-icon"><component :is="taskIcon(job)" :size="21" /></div>
            <div class="task-item-main">
              <div class="task-item-title">
                <strong>{{ taskName(job) }}</strong>
                <span :class="['task-status', statusMeta(job.status).tone]">
                  <component :is="statusMeta(job.status).icon" :size="13" :class="{ spin: activeStatuses.has(job.status) }" />
                  {{ statusMeta(job.status).label }}
                </span>
              </div>
              <p>{{ projectName(job) }} · {{ timeLabel(job.updatedAt) }}</p>
              <div v-if="activeStatuses.has(job.status)" class="task-progress">
                <i :style="{ width: `${Math.max(4, Math.round(job.progress * 100))}%` }" />
              </div>
              <p v-if="job.errorMessage" class="task-error-detail">{{ job.errorMessage }}</p>
              <div class="task-item-actions">
                <button type="button" @click="visit(job)">{{ job.status === "completed" ? "处理结果" : "查看" }}</button>
                <button
                  v-if="job.remoteJobId && activeStatuses.has(job.status)"
                  type="button"
                  class="cancel"
                  :disabled="Boolean(cancellingId)"
                  @click="cancel(job)"
                >{{ cancellingId === job.remoteJobId ? "取消中…" : "取消" }}</button>
              </div>
            </div>
          </article>
        </div>
      </aside>
    </div>
  </Teleport>
</template>
