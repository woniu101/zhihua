<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { compShareRepository, type CompShareInstance } from "../services/compShareRepository";
import { serviceRepository, type ServiceProbe } from "../services/serviceRepository";

const props = withDefaults(defineProps<{ context?: string; localOnly?: boolean }>(), {
  context: "需要生成时自动启动 GPU",
  localOnly: false,
});

const instance = ref<CompShareInstance>();
const probe = ref<ServiceProbe>();
const configured = ref<boolean>();
let refreshTimer: number | undefined;

const modeLabel = computed(() => {
  if (configured.value === false) return "未配置算力";
  if (!instance.value) return "实例状态未知";
  if (instance.value.runningMode === "gpu") return probe.value?.comfyuiReady ? "GPU 可生成" : "GPU 准备中";
  if (instance.value.runningMode === "noGpu") return "无卡运行";
  if (instance.value.runningMode === "stopped") return "已关机";
  if (instance.value.runningMode === "transitioning") return "状态切换中";
  return "实例状态未知";
});

const tone = computed(() => instance.value?.runningMode === "gpu" && probe.value?.comfyuiReady ? "ready" : instance.value?.state === "running" ? "online" : "idle");
const detail = computed(() => props.localOnly ? props.context : props.context);

async function refresh() {
  try {
    const configuration = await compShareRepository.configuration();
    configured.value = configuration.credentialsStored;
    if (!configuration.credentialsStored || !configuration.boundInstanceId) {
      instance.value = undefined;
      probe.value = undefined;
      return;
    }
    instance.value = await compShareRepository.boundInstance();
    if (instance.value.runningMode === "gpu") {
      probe.value = await serviceRepository.probe().catch(() => undefined);
    } else {
      probe.value = undefined;
    }
  } catch {
    configured.value = undefined;
    instance.value = undefined;
    probe.value = undefined;
  }
}

onMounted(() => {
  void refresh();
  refreshTimer = window.setInterval(refresh, 15_000);
  window.addEventListener("zhihua:compute-changed", refresh);
});
onBeforeUnmount(() => {
  if (refreshTimer !== undefined) window.clearInterval(refreshTimer);
  window.removeEventListener("zhihua:compute-changed", refresh);
});
</script>

<template>
  <div class="compute-status" :class="tone" :title="instance ? `${instance.region} · ${instance.zone} · ${instance.instanceId}` : modeLabel">
    <span class="compute-status-dot"></span>
    <strong>优云智算</strong>
    <b>{{ modeLabel }}</b>
    <span class="compute-status-divider">|</span>
    <span>{{ detail }}</span>
  </div>
</template>

<style scoped>
.compute-status{height:42px;padding:0 13px;border:1px solid var(--line);border-radius:9px;background:var(--surface);display:flex;align-items:center;gap:8px;color:var(--muted);font-size:13px;white-space:nowrap}.compute-status strong{color:var(--text)}.compute-status b{color:var(--muted)}.compute-status-dot{width:9px;height:9px;border-radius:50%;background:#98a8bd}.compute-status.online .compute-status-dot{background:#3686e8}.compute-status.ready .compute-status-dot{background:#13a36e}.compute-status.ready b{color:#087b53}.compute-status-divider{color:var(--line)}
</style>
