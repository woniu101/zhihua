<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { CalendarClock, Calculator, Database, ExternalLink, FolderOpen, HardDrive, KeyRound, Laptop, Monitor, Moon, Palette, Plus, Power, RefreshCw, Server, ShieldCheck, Sun, Wallet, Wrench, X } from "lucide-vue-next";
import {
  normalizeConnectionFailure,
  serviceRepository,
  type ServiceConnectionInfo,
  type ServiceProbe,
} from "../services/serviceRepository";
import {
  compShareRepository,
  normalizeCompShareError,
  type CompShareBalance,
  type CompShareConfiguration,
  type CompShareCreatePreflight,
  type CompShareCreateSpec,
  type CompShareInstance,
  type ComputeReleaseEligibility,
  type ComputeKeepAlivePolicy,
  type ComputePolicySnapshot,
  type ComputeWorkerReadiness,
  type ManagedComputeInstance,
} from "../services/compShareRepository";
import {
  sshTunnelRepository,
  type TunnelStatus,
} from "../services/sshTunnelRepository";
import {
  llmRepository,
  llmProviderPresets,
  normalizeLlmError,
  type LlmConfiguration,
  type LlmProviderId,
} from "../services/llmRepository";
import {
  effectiveTheme,
  setThemePreference,
  themePreference,
  type ThemePreference,
} from "../services/appearance";
import { storageRepository, type StorageInfo } from "../services/storageRepository";

const connectionInfo = ref<ServiceConnectionInfo>();
const probe = ref<ServiceProbe>();
const tunnelStatus = ref<TunnelStatus>({ configured: false, phase: "stopped" });
const computeOpen = ref(false);
const elasticCreateOpen = ref(false);
const llmOpen = ref(false);
const busy = ref(false);
const connectionError = ref("");
const computeForm = reactive({ publicKey: "", privateKey: "" });
const computeConfiguration = ref<CompShareConfiguration>();
const computeBalance = ref<CompShareBalance>();
const computeInstance = ref<CompShareInstance>();
const computeInstances = ref<CompShareInstance[]>([]);
const managedInstances = ref<ManagedComputeInstance[]>([]);
const workerReadiness = ref<Record<string, ComputeWorkerReadiness>>({});
const releaseEligibility = ref<Record<string, ComputeReleaseEligibility>>({});
const computePolicy = ref<ComputePolicySnapshot>({ policy: "economy", idleShutdownMinutes: 3, hardLimitMinutes: 60 });
const computeNotice = ref("");
const computeNoticeTone = ref<"success" | "error" | "neutral">("neutral");
const elasticCreateBusy = ref(false);
const elasticCreateNotice = ref("");
const elasticCreateNoticeTone = ref<"success" | "error" | "neutral">("neutral");
const elasticPreflight = ref<CompShareCreatePreflight>();
const elasticIdempotencyKeys = ref<string[]>([]);
const elasticForm = reactive({
  name: "",
  count: 1,
  region: "",
  zone: "",
  gpuType: "",
  cpu: 0,
  memoryGb: 0,
  imageId: "",
  projectId: "",
});
const llmConfiguration = ref<LlmConfiguration>();
const llmForm = reactive({ providerId: "deepseek" as LlmProviderId, baseUrl: "https://api.deepseek.com", model: "deepseek-chat", apiKey: "" });
const llmNotice = ref("");
const llmNoticeTone = ref<"success" | "error" | "neutral">("neutral");
const activeSection = ref<"service" | "overview" | "workers" | "connection" | "storage">("service");
const storageInfo = ref<StorageInfo>();
const storageNotice = ref("");
const computePolicyOptions: Array<{ value: ComputeKeepAlivePolicy; label: string; description: string; limit: string }> = [
  { value: "economy", label: "自动省心", description: "任务落盘后空闲 3 分钟关闭 GPU；关闭客户端且队列为空时立即关闭。", limit: "保护上限 1 小时" },
  { value: "availability", label: "连续创作", description: "空闲 15 分钟再关闭 GPU，适合连续制作多个镜头，减少重复准备。", limit: "保护上限 3 小时" },
  { value: "continuous", label: "保持算力", description: "空闲或关闭客户端后仍保持 GPU，适合库存紧张或持续工作。", limit: "保护上限 12 小时" },
];

const themeOptions: Array<{
  value: ThemePreference;
  label: string;
  description: string;
  icon: typeof Sun;
}> = [
  { value: "light", label: "浅色", description: "清透蓝灰背景与白色面板", icon: Sun },
  { value: "dark", label: "深色", description: "适合暗光环境和长时间编辑", icon: Moon },
  { value: "system", label: "跟随系统", description: "随 Windows 外观自动切换", icon: Laptop },
];

const computeConfigured = computed(() => Boolean(computeConfiguration.value?.credentialsStored));
const llmConfigured = computed(() => Boolean(
  llmConfiguration.value?.credentialStored
  && llmConfiguration.value.providerId === llmForm.providerId,
));
const selectedLlmPreset = computed(
  () => llmProviderPresets.find((item) => item.id === llmForm.providerId)
    ?? llmProviderPresets.find((item) => item.id === "custom")!,
);
const computeModeLabel = computed(() => {
  const mode = computeInstance.value?.runningMode;
  if (mode === "gpu") return "GPU 运行中";
  if (mode === "noGpu") return "无卡运行";
  if (mode === "stopped") return "已关机";
  if (mode === "transitioning") return "状态切换中";
  return "等待查询";
});
const stopSchedulerLabel = computed(() => {
  const timestamp = computeInstance.value?.stopSchedulerTime;
  if (!timestamp) return "当前未设置平台定时关机";
  return `平台将在 ${new Date(timestamp * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  })} 自动关机`;
});
const policyGuardLabel = computed(() => {
  if (computeInstance.value?.runningMode !== "gpu") return "启动 GPU 时生效";
  return computeInstance.value.stopSchedulerTime ? "平台兜底已启用" : "保障待设置";
});
const selectedManagedInstance = computed(() => managedInstances.value.find(
  (item) => item.instanceId === computeConfiguration.value?.boundInstanceId,
));
const releaseTimeLabel = computed(() => {
  const timestamp = selectedManagedInstance.value?.releaseTime;
  if (!timestamp) return "平台暂未返回回收时间";
  return new Date(timestamp * 1000).toLocaleString("zh-CN", {
    year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit",
  });
});
const gpuWorkerCount = computed(() => managedInstances.value.filter((item) => item.runningMode === "gpu").length);
const noGpuWorkerCount = computed(() => managedInstances.value.filter((item) => item.runningMode === "no_gpu").length);
const stoppedWorkerCount = computed(() => managedInstances.value.filter((item) => item.runningMode === "stopped").length);
const readyWorkerCount = computed(() => managedInstances.value.filter((item) => workerReadiness.value[item.instanceId]?.state === "ready").length);
const activeWorkerPrice = computed(() => managedInstances.value.reduce(
  (total, item) => total + (item.runningMode === "gpu" ? item.instancePrice ?? 0 : 0),
  0,
));
const activeWorkerJobs = computed(() => managedInstances.value.filter((item) => Boolean(item.currentJobId)).length);

const connected = computed(
  () => Boolean(connectionInfo.value?.configured && probe.value?.compatible),
);
const connectionLabel = computed(() => {
  if (tunnelStatus.value.phase === "connecting") return "正在建立安全连接";
  if (tunnelStatus.value.phase === "reconnecting") return "安全连接正在恢复";
  if (tunnelStatus.value.phase === "error") return "安全连接异常";
  if (!connectionInfo.value?.configured) return "尚未配置";
  if (connected.value) return "连接正常";
  return "等待连接";
});
const serviceStatus = computed(() => {
  if (connected.value) {
    return probe.value?.comfyuiReady
      ? "知画服务与 ComfyUI 已就绪"
      : "知画服务已连接 · 无卡维护模式";
  }
  return connectionInfo.value?.configured
    ? "知画服务暂不可达"
    : "请先配置知画服务";
});
const simpleServiceState = computed(() => {
  if (!computeConfigured.value) return { tone: "attention", title: "还没有连接生成算力", detail: "连接优云智算账户后，知画会在生成时自动准备 GPU。", action: "连接生成服务" };
  if (!computeInstance.value) return { tone: "attention", title: "请选择一台生成实例", detail: "选择已有实例即可，知画不会自动释放用户创建的实例。", action: "选择实例" };
  if (computeInstance.value.runningMode === "gpu" && probe.value?.comfyuiReady) return { tone: "ready", title: "生成服务已准备好", detail: `${activeWorkerJobs.value} 个任务执行中，${probe.value?.queueQueued ?? 0} 个等待。`, action: "重新检查" };
  if (computeInstance.value.runningMode === "gpu") return { tone: "working", title: "GPU 正在准备生成服务", detail: "环境检查完成后即可生成，任务状态会持续保留。", action: "检查进度" };
  return { tone: "idle", title: "需要生成时再启动 GPU", detail: "当前不产生 GPU 运行费；提交任务时知画也会自动启动。", action: "检查并准备" };
});
const simplePolicyLabel = computed(() => ({
  economy: "任务完成 3 分钟后关闭",
  availability: "任务完成 15 分钟后关闭",
  continuous: "持续运行至保护上限",
}[computePolicy.value.policy]));

async function prepareGenerationService() {
  if (!computeConfigured.value || !computeInstance.value) {
    computeOpen.value = true;
    return;
  }
  if (computeInstance.value.runningMode === "gpu") {
    await Promise.allSettled([refreshConnection(), refreshCompute()]);
    return;
  }
  await changeComputeMode("gpu");
}
const checks = computed(() => [
  {
    name: "知画后端",
    state: connected.value ? `已连接 · v${probe.value?.serviceVersion}` : "等待连接",
    tone: connected.value ? "success" : "waiting",
  },
  {
    name: "ComfyUI",
    state: probe.value?.comfyuiReady
      ? "可生成"
      : probe.value?.comfyuiConnected
        ? "已连接，等待就绪"
        : "启动 GPU 后检查",
    tone: probe.value?.comfyuiReady ? "success" : "waiting",
  },
  {
    name: "工作流清单",
    state: connected.value
      ? `${probe.value?.availableWorkflows.length ?? 0} / ${probe.value?.workflows.length ?? 0} 个模板已安装`
      : "等待握手",
    tone: probe.value?.availableWorkflows.length ? "success" : "waiting",
  },
  {
    name: "模型清单",
    state: connected.value ? probe.value?.modelManifestVersion ?? "未知" : "等待握手",
    tone: connected.value ? "success" : "waiting",
  },
  {
    name: "任务队列",
    state: connected.value
      ? `${probe.value?.queueActive ?? 0} 个执行中 · ${probe.value?.queueQueued ?? 0} 个等待`
      : "等待连接",
    tone: connected.value ? "success" : "waiting",
  },
  {
    name: "本机安全入口",
    state: tunnelStatus.value.phase === "connected"
      ? `SSH 已连接 · ${tunnelStatus.value.localUrl ?? "本机随机端口"}`
      : tunnelStatus.value.lastError ?? (tunnelStatus.value.configured ? "等待建立 SSH 连接" : "尚未配置 SSH 连接"),
    tone: tunnelStatus.value.phase === "connected" ? "success" : "waiting",
  },
]);

function setComputeNotice(message: string, tone: "success" | "error" | "neutral") {
  computeNotice.value = message;
  computeNoticeTone.value = tone;
}

const elasticTemplateAvailable = computed(() => Boolean(
  computeInstance.value?.region
  && computeInstance.value.zone
  && computeInstance.value.gpuType
  && computeInstance.value.cpu
  && computeInstance.value.memoryMb
  && computeInstance.value.imageId,
));
const elasticCreateCount = computed(() => Math.min(50, Math.max(1, Math.round(elasticForm.count || 1))));
const elasticTotalHourlyPrice = computed(() => {
  const price = elasticPreflight.value?.estimatedHourlyPrice;
  return price == null ? undefined : price * elasticCreateCount.value;
});

function resetElasticIdempotencyKeys() {
  elasticIdempotencyKeys.value = Array.from(
    { length: elasticCreateCount.value },
    () => crypto.randomUUID(),
  );
}

function elasticCreateSpec(): CompShareCreateSpec {
  return {
    region: elasticForm.region.trim(),
    zone: elasticForm.zone.trim(),
    gpuType: elasticForm.gpuType.trim(),
    gpuCount: 1,
    cpu: Math.max(1, Math.round(elasticForm.cpu)),
    memoryMb: Math.max(1, Math.round(elasticForm.memoryGb * 1024)),
    imageId: elasticForm.imageId.trim(),
    chargeType: "Postpay",
    projectId: elasticForm.projectId.trim() || undefined,
  };
}

function openElasticCreate() {
  const template = computeInstance.value;
  if (!template || !elasticTemplateAvailable.value) {
    setComputeNotice("主实例缺少地域、GPU、CPU、内存或镜像信息，当前不能生成可核验的扩容方案。", "error");
    return;
  }
  Object.assign(elasticForm, {
    name: `zhihua-elastic-${new Date().toISOString().slice(5, 16).replace(/[-T:]/g, "")}`,
    count: 1,
    region: template.region,
    zone: template.zone,
    gpuType: template.gpuType ?? "",
    cpu: template.cpu ?? 0,
    memoryGb: Math.round((template.memoryMb ?? 0) / 1024),
    imageId: template.imageId ?? "",
    projectId: template.projectId ?? "",
  });
  elasticPreflight.value = undefined;
  resetElasticIdempotencyKeys();
  elasticCreateNotice.value = "先查询实时库存与按量报价；预检不会创建实例或产生 GPU 费用。";
  elasticCreateNoticeTone.value = "neutral";
  computeOpen.value = false;
  elasticCreateOpen.value = true;
}

function closeElasticCreate() {
  if (elasticCreateBusy.value) return;
  elasticCreateOpen.value = false;
  computeOpen.value = true;
}

async function preflightElasticCreate() {
  elasticCreateBusy.value = true;
  elasticCreateNotice.value = "正在查询实时库存和报价…";
  elasticCreateNoticeTone.value = "neutral";
  try {
    elasticPreflight.value = await compShareRepository.preflightCreate(elasticCreateSpec());
    elasticCreateNotice.value = elasticPreflight.value.capacityAvailable
      ? "预检通过。请核对下方地域、规格、镜像和费率，再确认创建。"
      : "当前没有完全匹配的空闲实例，未创建任何资源。";
    elasticCreateNoticeTone.value = elasticPreflight.value.capacityAvailable ? "success" : "error";
  } catch (error) {
    elasticPreflight.value = undefined;
    elasticCreateNotice.value = normalizeCompShareError(error).message;
    elasticCreateNoticeTone.value = "error";
  } finally {
    elasticCreateBusy.value = false;
  }
}

async function confirmElasticCreate() {
  if (!elasticPreflight.value?.capacityAvailable || elasticCreateBusy.value) return;
  elasticCreateBusy.value = true;
  const count = elasticCreateCount.value;
  elasticCreateNotice.value = `已确认费用，正在提交 ${count} 个相互独立的幂等创建请求…`;
  elasticCreateNoticeTone.value = "neutral";
  try {
    if (elasticIdempotencyKeys.value.length !== count) resetElasticIdempotencyKeys();
    const baseName = elasticForm.name.trim();
    const requests = elasticIdempotencyKeys.value.map((idempotencyKey, index) => {
      const suffix = count > 1 ? `-${String(index + 1).padStart(2, "0")}` : "";
      return compShareRepository.createManagedInstance({
        idempotencyKey,
        name: `${baseName.slice(0, 63 - suffix.length)}${suffix}`,
        spec: elasticCreateSpec(),
        role: "elastic",
        confirmed: true,
      });
    });
    const results = await Promise.allSettled(requests);
    const operations = results.flatMap((result) => result.status === "fulfilled" ? [result.value] : []);
    const succeeded = operations.filter((item) => item.status === "succeeded").length;
    const unknown = operations.filter((item) => item.status === "unknown").length;
    const failed = count - succeeded - unknown;
    await refreshCompute();
    if (succeeded === count) {
      elasticCreateOpen.value = false;
      computeOpen.value = true;
      setComputeNotice(`${succeeded} 个弹性实例已创建。服务准备完成前不会领取生成任务。`, "success");
    } else {
      elasticCreateNotice.value = `扩容结果：成功 ${succeeded}，待对账 ${unknown}，失败 ${failed}。不会自动补购；请先刷新实例中心核对。`;
      elasticCreateNoticeTone.value = failed > 0 ? "error" : "neutral";
    }
  } catch (error) {
    elasticCreateNotice.value = normalizeCompShareError(error).message;
    elasticCreateNoticeTone.value = "error";
  } finally {
    elasticCreateBusy.value = false;
  }
}

watch(
  () => [elasticForm.count, elasticForm.region, elasticForm.zone, elasticForm.gpuType, elasticForm.cpu, elasticForm.memoryGb, elasticForm.imageId, elasticForm.projectId],
  () => {
    if (elasticPreflight.value) {
      elasticPreflight.value = undefined;
      resetElasticIdempotencyKeys();
      elasticCreateNotice.value = "规格已经变化，请重新查询实时库存和报价。";
      elasticCreateNoticeTone.value = "neutral";
    }
  },
);

async function refreshCompute() {
  try {
    computePolicy.value = await compShareRepository.computePolicy();
    computeConfiguration.value = await compShareRepository.configuration();
    if (!computeConfiguration.value.credentialsStored) return;
    const [balance, instances, managed, readiness] = await Promise.all([
      compShareRepository.balance(),
      compShareRepository.listInstances(),
      compShareRepository.reconcileInstances(),
      compShareRepository.workerReadiness(),
    ]);
    computeBalance.value = balance;
    computeInstances.value = instances;
    managedInstances.value = managed;
    workerReadiness.value = Object.fromEntries(readiness.map((item) => [item.instanceId, item]));
    const entries = await Promise.all(managed.map(async (item) => [
      item.instanceId,
      await compShareRepository.releaseEligibility(item.instanceId),
    ] as const));
    releaseEligibility.value = Object.fromEntries(entries);
    const boundId = computeConfiguration.value.boundInstanceId;
    computeInstance.value = boundId
      ? instances.find((item) => item.instanceId === boundId)
      : undefined;
    if (boundId && !computeInstance.value) {
      computeInstance.value = await compShareRepository.boundInstance();
    }
  } catch (error) {
    setComputeNotice(normalizeCompShareError(error).message, "error");
  }
}

async function changeComputePolicy(policy: ComputeKeepAlivePolicy) {
  busy.value = true;
  try {
    computePolicy.value = await compShareRepository.setComputePolicy(policy);
    await refreshCompute();
    const selected = computePolicyOptions.find((item) => item.value === policy)!;
    setComputeNotice(`已切换为“${selected.label}”，${selected.limit}的平台关机保障已生效。`, "success");
  } catch (error) {
    setComputeNotice(normalizeCompShareError(error).message, "error");
  } finally {
    busy.value = false;
  }
}

async function saveComputeCredentials() {
  if (!computeForm.publicKey.trim() || !computeForm.privateKey.trim()) {
    setComputeNotice("请填写 API 公钥和私钥。", "error");
    return;
  }
  busy.value = true;
  setComputeNotice("正在验证优云智算账户……", "neutral");
  try {
    computeConfiguration.value = await compShareRepository.saveCredentials(
      computeForm.publicKey.trim(),
      computeForm.privateKey.trim(),
    );
    const tested = await compShareRepository.testConnection();
    computeBalance.value = tested.balance;
    computeForm.publicKey = "";
    computeForm.privateKey = "";
    computeInstances.value = await compShareRepository.listInstances();
    managedInstances.value = await compShareRepository.reconcileInstances();
    setComputeNotice("账户验证成功，密钥已保存到 Windows 凭据管理器。", "success");
  } catch (error) {
    setComputeNotice(normalizeCompShareError(error).message, "error");
  } finally {
    busy.value = false;
  }
}

function roleLabel(role: ManagedComputeInstance["role"]) {
  return role === "primary" ? "主实例" : role === "elastic" ? "弹性实例" : role === "test" ? "测试实例" : "用户实例";
}

function managementLabel(item: ManagedComputeInstance) {
  if (item.ownership === "zhihua_managed") return "知画创建并管理";
  if (item.role === "elastic" || item.role === "test") return "用户创建 · 已授权自动启停";
  if (item.role === "primary") return "用户创建 · 当前主实例";
  return "用户创建 · 仅手动控制";
}

function workerReadinessLabel(instanceId: string) {
  const state = workerReadiness.value[instanceId]?.state;
  if (state === "ready") return "服务就绪";
  if (state === "connecting") return "正在检查";
  if (state === "waiting_for_gpu") return "等待 GPU";
  if (state === "incompatible") return "版本不兼容";
  if (state === "unreachable") return "服务不可达";
  return "服务未检查";
}

function workerReadinessTone(instanceId: string) {
  const state = workerReadiness.value[instanceId]?.state;
  return state === "ready" ? "ready" : state === "incompatible" || state === "unreachable" ? "error" : "pending";
}

function managedModeLabel(item: ManagedComputeInstance) {
  if (item.runningMode === "gpu") return "GPU 运行";
  if (item.runningMode === "no_gpu") return "无卡运行";
  if (item.runningMode === "stopped") return "已关机";
  if (item.lifecycleState === "unknown") return "待平台对账";
  return item.platformState;
}

function formatReleaseTime(timestamp?: number) {
  if (!timestamp) return "回收时间未返回";
  return `预计回收 ${new Date(timestamp * 1000).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" })}`;
}

async function releaseManagedInstance(item: ManagedComputeInstance) {
  const eligibility = releaseEligibility.value[item.instanceId];
  if (!eligibility?.allowed) {
    setComputeNotice(eligibility?.reasons.join("；") || "该实例当前不能释放。", "error");
    return;
  }
  if (!window.confirm(`确认释放“${item.name ?? item.instanceId}”吗？实例将进入优云智算回收流程，数据处理以平台规则为准。`)) return;
  busy.value = true;
  try {
    const operation = await compShareRepository.releaseManagedInstance(item.instanceId);
    if (operation.status === "succeeded") {
      setComputeNotice("实例已释放。", "success");
    } else if (operation.status === "unknown") {
      setComputeNotice("平台响应结果不确定，已停止重试并等待下一次对账。", "neutral");
    } else {
      setComputeNotice(operation.errorMessage ?? "实例释放失败。", "error");
    }
    await refreshCompute();
  } catch (error) {
    setComputeNotice(normalizeCompShareError(error).message, "error");
  } finally {
    busy.value = false;
  }
}

async function bindComputeInstance(instance: CompShareInstance) {
  busy.value = true;
  try {
    computeInstance.value = await compShareRepository.bindInstance(instance);
    computeConfiguration.value = await compShareRepository.configuration();
    setComputeNotice(`已绑定 ${instance.name ?? instance.instanceId}。`, "success");
  } catch (error) {
    setComputeNotice(normalizeCompShareError(error).message, "error");
  } finally {
    busy.value = false;
  }
}

async function bindManagedInstance(item: ManagedComputeInstance) {
  const platformInstance = computeInstances.value.find((candidate) => candidate.instanceId === item.instanceId);
  if (!platformInstance) {
    setComputeNotice("该实例尚未出现在本次平台查询中，完成对账前不能设为主实例。", "error");
    return;
  }
  await bindComputeInstance(platformInstance);
  await refreshCompute();
}

async function setUserWorkerEnabled(item: ManagedComputeInstance, enabled: boolean) {
  busy.value = true;
  setComputeNotice(
    enabled
      ? `正在把“${item.name ?? item.instanceId}”加入批量算力池……`
      : `正在把“${item.name ?? item.instanceId}”移出批量算力池……`,
    "neutral",
  );
  try {
    await compShareRepository.setUserWorkerEnabled(item.instanceId, enabled);
    await refreshCompute();
    setComputeNotice(
      enabled
        ? "已授权知画按任务自动启停该实例；实例仍归用户所有，知画不会释放它。首次使用前还需完成独立 SSH 连接。"
        : "已移出批量算力池；知画不会再自动启停该实例。",
      "success",
    );
  } catch (error) {
    setComputeNotice(normalizeCompShareError(error).message, "error");
    await refreshCompute();
  } finally {
    busy.value = false;
  }
}

async function checkManagedWorker(item: ManagedComputeInstance) {
  busy.value = true;
  setComputeNotice(`正在连接“${item.name ?? item.instanceId}”并检查知画服务……`, "neutral");
  try {
    const result = await serviceRepository.connectWorker(item.instanceId);
    if (!result) throw new Error("知画服务没有返回检查结果。");
    await refreshCompute();
    setComputeNotice(
      result.comfyuiReady
        ? `“${item.name ?? item.instanceId}”的知画服务与 ComfyUI 已就绪。`
        : `“${item.name ?? item.instanceId}”的基础服务可达，等待 GPU 后即可生成。`,
      result.comfyuiReady ? "success" : "neutral",
    );
  } catch (error) {
    setComputeNotice(normalizeConnectionFailure(error).message, "error");
    await refreshCompute();
  } finally {
    busy.value = false;
  }
}

async function provisionManagedWorkerConnection(item: ManagedComputeInstance) {
  busy.value = true;
  setComputeNotice(
    `正在为“${item.name ?? item.instanceId}”配置安全连接；如实例已关机，只会无卡启动……`,
    "neutral",
  );
  try {
    const result = await serviceRepository.provisionWorkerConnection(item.instanceId);
    if (!result) throw new Error("知画服务没有返回连接结果。");
    if (item.instanceId === computeConfiguration.value?.boundInstanceId) probe.value = result;
    await refreshCompute();
    setComputeNotice(
      result.comfyuiReady
        ? `“${item.name ?? item.instanceId}”的安全连接已配置，GPU 生成服务可用。`
        : `“${item.name ?? item.instanceId}”的安全连接已配置，实例保持无卡维护模式。`,
      "success",
    );
  } catch (error) {
    setComputeNotice(normalizeConnectionFailure(error).message, "error");
    await refreshCompute();
  } finally {
    busy.value = false;
  }
}

async function prepareManagedWorker(item: ManagedComputeInstance) {
  busy.value = true;
  setComputeNotice(`正在启动“${item.name ?? item.instanceId}”并准备生成服务……`, "neutral");
  try {
    const result = await serviceRepository.prepareWorker(item.instanceId);
    if (!result?.comfyuiReady) throw new Error("GPU 已启动，但生成服务尚未就绪。");
    probe.value = result;
    await refreshCompute();
    setComputeNotice(`“${item.name ?? item.instanceId}”已成为可用 GPU worker。`, "success");
  } catch (error) {
    setComputeNotice(normalizeConnectionFailure(error).message, "error");
    await refreshCompute();
  } finally {
    busy.value = false;
  }
}

async function changeComputeMode(mode: "gpu" | "noGpu" | "stop") {
  busy.value = true;
  try {
    if (mode === "gpu") {
      const instanceId = computeConfiguration.value?.boundInstanceId;
      if (!instanceId) throw new Error("请先选择主实例。");
      probe.value = await serviceRepository.prepareWorker(instanceId);
      await refreshCompute();
      setComputeNotice("GPU 与生成服务已就绪。", "success");
      return;
    }
    const result = mode === "stop"
      ? await compShareRepository.stop()
      : await compShareRepository.start(mode);
    computeInstance.value = result.instance;
    setComputeNotice(
      mode === "noGpu" ? "已请求无卡启动。" : "已请求关机。",
      "success",
    );
    window.setTimeout(refreshCompute, 3500);
  } catch (error) {
    setComputeNotice(normalizeCompShareError(error).message, "error");
  } finally {
    busy.value = false;
  }
}

async function toggleStopDeadline() {
  if (!computeInstance.value || computeInstance.value.state !== "running") return;
  busy.value = true;
  try {
    const result = await compShareRepository.setStopDeadline(
      Math.floor(Date.now() / 1000) + computePolicy.value.hardLimitMinutes * 60,
      computeConfiguration.value?.projectId,
    );
    computeInstance.value = result.instance;
    setComputeNotice(`已刷新 ${computePolicy.value.hardLimitMinutes} 分钟平台定时关机保障。`, "success");
  } catch (error) {
    setComputeNotice(normalizeCompShareError(error).message, "error");
  } finally {
    busy.value = false;
  }
}

async function refreshConnection() {
  try {
    tunnelStatus.value = await sshTunnelRepository.status();
    if (tunnelStatus.value.configured) {
      probe.value = await sshTunnelRepository.connectService();
      tunnelStatus.value = await sshTunnelRepository.status();
    }
    connectionInfo.value = await serviceRepository.info();
    if (connectionInfo.value?.configured && !probe.value) {
      probe.value = await serviceRepository.probe();
    }
    connectionError.value = "";
  } catch (error) {
    tunnelStatus.value = await sshTunnelRepository.status().catch(() => ({
      configured: false,
      phase: "error" as const,
    }));
    probe.value = undefined;
    connectionError.value = normalizeConnectionFailure(error).message;
    connectionInfo.value = await serviceRepository.info().catch(() => undefined);
  }
}

async function refreshLlm() {
  try {
    llmConfiguration.value = await llmRepository.configuration();
    llmForm.providerId = llmConfiguration.value.providerId;
    llmForm.baseUrl = llmConfiguration.value.baseUrl;
    llmForm.model = llmConfiguration.value.model;
  } catch (error) {
    llmNotice.value = normalizeLlmError(error).message;
    llmNoticeTone.value = "error";
  }
}

async function saveOrTestLlm() {
  busy.value = true;
  llmNotice.value = "正在验证候选配置；验证通过后才会保存……";
  llmNoticeTone.value = "neutral";
  try {
    llmConfiguration.value = await llmRepository.save(
      llmForm.providerId,
      llmForm.baseUrl,
      llmForm.model,
      llmForm.apiKey.trim(),
    );
    llmForm.apiKey = "";
    llmNotice.value = `连接成功，内容规划将使用 ${llmConfiguration.value.providerLabel} · ${llmConfiguration.value.model}。`;
    llmNoticeTone.value = "success";
  } catch (error) {
    llmNotice.value = normalizeLlmError(error).message;
    llmNoticeTone.value = "error";
  } finally {
    busy.value = false;
  }
}

function selectLlmProvider(event: Event) {
  const providerId = (event.target as HTMLSelectElement).value as LlmProviderId;
  const preset = llmProviderPresets.find((item) => item.id === providerId);
  if (!preset) return;
  llmForm.providerId = providerId;
  llmForm.baseUrl = preset.baseUrl;
  llmForm.model = preset.model;
  llmForm.apiKey = "";
  llmNotice.value = providerId === "doubao" ? "豆包的模型字段填写方舟控制台中的推理接入点 ID。" : "";
  llmNoticeTone.value = "neutral";
}

async function clearLlm() {
  await llmRepository.clear();
  await refreshLlm();
  llmNotice.value = "已从 Windows 凭据管理器移除大模型 API Key。";
  llmNoticeTone.value = "neutral";
}

async function refreshStorage() {
  storageNotice.value = "";
  try {
    storageInfo.value = await storageRepository.info();
  } catch (error) {
    storageNotice.value = normalizeConnectionFailure(error).message;
  }
}

async function openProjectsRoot() {
  try {
    await storageRepository.openProjectsRoot();
  } catch (error) {
    storageNotice.value = normalizeConnectionFailure(error).message;
  }
}

async function chooseProjectsRoot() {
  storageNotice.value = "";
  try {
    const result = await storageRepository.chooseProjectsRoot();
    if (!result) return;
    storageInfo.value = result;
    storageNotice.value = result.restartRequired
      ? "新项目目录已保存。重启知画后生效，已有项目仍保留在原位置。"
      : "项目目录已更新。";
  } catch (error) {
    storageNotice.value = normalizeConnectionFailure(error).message;
  }
}

async function resetProjectsRoot() {
  storageNotice.value = "";
  try {
    const result = await storageRepository.resetProjectsRoot();
    if (!result) return;
    storageInfo.value = result;
    storageNotice.value = result.restartRequired
      ? "已恢复默认项目目录，重启知画后生效。"
      : "当前已经使用默认项目目录。";
  } catch (error) {
    storageNotice.value = normalizeConnectionFailure(error).message;
  }
}

onMounted(() => Promise.allSettled([refreshConnection(), refreshCompute(), refreshLlm(), refreshStorage()]));
</script>

<template>
  <section class="page settings-page">
    <header class="settings-head"><div class="page-title-line"><h1>设置</h1><span class="status-line"><span class="dot" :class="{ gray: !connected }"></span><strong>{{ serviceStatus }}</strong><span>|</span><span>本地项目自动保存</span></span></div></header>
    <nav class="settings-tabs">
      <button :class="{ active: activeSection === 'service' }" type="button" @click="activeSection='service'"><Server :size="20"/>生成服务</button>
      <button :class="{ active: activeSection === 'storage' }" type="button" @click="activeSection='storage'"><Monitor :size="20"/>存储与外观</button>
      <button type="button" @click="llmOpen=true"><KeyRound :size="20"/>大模型设置</button>
      <span class="settings-tabs-spacer"></span>
      <span class="advanced-label">高级管理</span>
      <button :class="{ active: activeSection === 'overview' }" type="button" @click="activeSection='overview'"><Calculator :size="20"/>费用与保护</button>
      <button :class="{ active: activeSection === 'workers' }" type="button" @click="activeSection='workers'"><Database :size="20"/>实例与并行 <i v-if="managedInstances.length">{{ managedInstances.length }}</i></button>
      <button :class="{ active: activeSection === 'connection' }" type="button" @click="activeSection='connection'"><ShieldCheck :size="20"/>连接诊断</button>
    </nav>

    <div v-if="activeSection === 'service'" class="service-page">
      <section class="panel service-hero" :class="`tone-${simpleServiceState.tone}`">
        <div class="service-symbol"><Server :size="34"/></div>
        <div class="service-copy"><span class="eyebrow">生成服务</span><h2>{{ simpleServiceState.title }}</h2><p>{{ simpleServiceState.detail }}</p></div>
        <div class="service-cost"><small>当前预计费率</small><b>{{ computeInstance?.runningMode === 'gpu' ? `¥ ${activeWorkerPrice.toFixed(2)}/小时` : '当前不计 GPU 费' }}</b><span>余额 {{ computeBalance?.amountAvailable ? `¥${computeBalance.amountAvailable}` : '等待查询' }}</span></div>
        <button class="btn primary large" type="button" :disabled="busy" @click="prepareGenerationService"><RefreshCw v-if="busy" class="spin" :size="19"/><Power v-else :size="19"/>{{ simpleServiceState.action }}</button>
      </section>
      <section class="service-cards">
        <article class="panel service-card"><div class="service-card-icon"><CalendarClock :size="25"/></div><div><small>生成后多久关闭</small><h3>{{ simplePolicyLabel }}</h3><p>默认自动省心，防止客户端退出后忘记关机。</p></div><button class="btn" type="button" @click="activeSection='overview'">调整</button></article>
        <article class="panel service-card"><div class="service-card-icon"><Database :size="25"/></div><div><small>并行生成能力</small><h3>{{ managedInstances.length > 1 ? `最多可用 ${managedInstances.length} 路` : '当前适合单路生成' }}</h3><p>生成多个镜头时可选择省钱排队、智能安排或加速并行。</p></div><button class="btn" type="button" @click="activeSection='workers'">查看</button></article>
        <article class="panel service-card"><div class="service-card-icon"><ShieldCheck :size="25"/></div><div><small>连接与环境</small><h3>{{ connected ? '检查通过' : '需要检查' }}</h3><p>知画会自动处理安全连接，详细诊断只在异常时需要。</p></div><button class="btn" type="button" @click="activeSection='connection'">诊断</button></article>
      </section>
      <section class="panel simple-explainer"><div><h2>平时不需要管理实例</h2><p>在分镜页提交生成时，知画会检查服务、启动 GPU、保存任务，并按你选择的策略自动关闭。高级管理保留实例、并行、连接和生命周期的完整控制。</p></div><button class="btn" type="button" @click="activeSection='workers'">进入高级管理</button></section>
    </div>

    <div v-else-if="activeSection === 'overview'" class="settings-grid settings-overview">
      <section class="panel account overview-metrics">
        <div class="panel-head">
          <div><h2>算力概览</h2><p>余额、GPU、费用与任务状态</p></div>
          <div class="overview-actions">
            <button class="btn link" type="button" :disabled="busy" @click="refreshCompute"><RefreshCw :size="17"/>实时刷新</button>
            <button class="btn link" type="button" @click="computeOpen=true"><Wallet :size="18"/>管理算力账户</button>
          </div>
        </div>
        <div class="overview-stat-grid">
          <article>
            <span class="overview-stat-icon"><Database :size="27"/></span>
            <div><small>可用余额</small><b>{{ computeBalance?.amountAvailable ? `¥ ${computeBalance.amountAvailable}` : '--' }}</b><p>以优云智算账户为准</p></div>
          </article>
          <article>
            <span class="overview-stat-icon"><Server :size="27"/></span>
            <div><small>GPU 状态</small><b>{{ gpuWorkerCount }} 台运行</b><p>{{ noGpuWorkerCount }} 台无卡 · {{ stoppedWorkerCount }} 台关机</p></div>
          </article>
          <article>
            <span class="overview-stat-icon"><Calculator :size="27"/></span>
            <div><small>当前预估费率</small><b>{{ `¥ ${activeWorkerPrice.toFixed(2)}/小时` }}</b><p>仅统计正在运行的 GPU</p></div>
          </article>
          <article>
            <span class="overview-stat-icon"><ShieldCheck :size="27"/></span>
            <div><small>生成任务</small><b>{{ activeWorkerJobs }} 个执行中</b><p>{{ probe?.queueQueued ?? 0 }} 个等待 · {{ readyWorkerCount }} 台服务就绪</p></div>
          </article>
        </div>
      </section>

      <section class="panel instance"><div class="panel-head"><h2>主实例</h2><span :class="computeInstance?.state === 'running' ? 'success-text' : 'waiting-text'"><span class="dot" :class="{ gray: computeInstance?.state !== 'running' }"></span>{{ computeModeLabel }}</span></div><div class="instance-main"><div class="server-art">▤</div><div class="instance-identity"><h2 :title="computeInstance?.name">{{ computeInstance?.name ?? '尚未绑定实例' }}</h2><p>{{ computeInstance?.gpuType ? `RTX ${computeInstance.gpuType}` : '等待实例信息' }}</p><span class="muted"><span class="dot" :class="{ gray: !probe?.comfyuiReady }"></span>{{ probe?.comfyuiReady ? '生成服务可用' : '生成服务待启动' }}　·　{{ computeInstance ? `${computeInstance.cpu ?? '--'} 核 / ${computeInstance.memoryMb ? Math.round(computeInstance.memoryMb / 1024) : '--'} GB` : '等待配置' }}</span></div></div><div class="instance-metrics"><div><span>当前模式</span><b>{{ computeModeLabel }}</b></div><div><span>GPU 规格</span><b>{{ computeInstance?.gpuType ? `RTX ${computeInstance.gpuType}` : '--' }}</b></div><div><span>区域/可用区</span><b>{{ computeInstance?.zone ?? '--' }}</b></div></div><div class="instance-actions"><button class="btn primary" type="button" :disabled="busy || computeInstance?.state !== 'stopped'" @click="changeComputeMode('gpu')"><Power :size="18"/>启动 GPU</button><button class="btn" type="button" :disabled="busy || !computeInstance || (computeInstance.state !== 'stopped' && computeInstance.state !== 'running')" @click="computeInstance?.state === 'running' ? changeComputeMode('stop') : changeComputeMode('noGpu')"><Wrench :size="18"/>{{ computeInstance?.state === 'running' ? '关机' : '无卡启动' }}</button><button class="btn" type="button" @click="activeSection='workers'"><ExternalLink :size="17"/>实例池</button></div></section>

      <section class="panel shutdown"><div class="panel-title"><h2>生成后多久关闭</h2><span class="policy-badge" :class="{ neutral: computeInstance?.runningMode !== 'gpu' || !computeInstance?.stopSchedulerTime }">{{ policyGuardLabel }}</span></div><div class="policy-options"><button v-for="option in computePolicyOptions" :key="option.value" type="button" :class="{ selected: computePolicy.policy === option.value }" :disabled="busy" @click="changeComputePolicy(option.value)"><span><b>{{ option.label }}</b><small>{{ option.limit }}</small></span><p>{{ option.description }}</p><i>{{ computePolicy.policy === option.value ? '✓' : '' }}</i></button></div><div class="policy-guard"><CalendarClock :size="21"/><span><b>平台硬保护</b><small>{{ stopSchedulerLabel }}</small></span><button type="button" :disabled="busy || computeInstance?.state !== 'running' || computeInstance?.runningMode !== 'gpu'" @click="toggleStopDeadline">刷新保障</button></div></section>

      <section class="panel retention overview-retention">
        <div class="panel-title"><h2>费用与生命周期保障</h2><span class="policy-badge neutral">逐实例执行</span></div>
        <div class="lifecycle-list">
          <article><CalendarClock :size="22"/><div><b>主实例回收时间</b><span>{{ releaseTimeLabel }}</span></div></article>
          <article><ShieldCheck :size="22"/><div><b>用户实例永久保留</b><span>知画只按策略关闭 GPU，不会自动释放用户创建的实例。</span></div></article>
          <article><Server :size="22"/><div><b>弹性实例受保护</b><span>仅在任务与结果已落盘、实例已关机并确认后允许释放。</span></div></article>
        </div>
        <div class="warning-box compact"><b>平台状态未知时暂停回收</b><span>先重新同步状态，避免误关任务或产生遗留费用。</span></div>
        <button class="btn worker-link" type="button" @click="activeSection='workers'">查看每台实例的状态与保护</button>
      </section>
    </div>

    <div v-else-if="activeSection === 'workers'" class="worker-page">
      <section class="worker-summary">
        <article><span>可参与并行</span><b>{{ managedInstances.length }}</b><small>每台实例可以同时处理一路视频</small></article>
        <article><span>GPU 运行</span><b>{{ gpuWorkerCount }}</b><small>约 ¥{{ activeWorkerPrice.toFixed(2) }}/小时</small></article>
        <article><span>无卡 / 关机</span><b>{{ noGpuWorkerCount }} / {{ stoppedWorkerCount }}</b><small>保留环境，停止 GPU 计费</small></article>
        <article><span>服务就绪</span><b>{{ readyWorkerCount }}</b><small>{{ activeWorkerJobs }} 个任务执行中</small></article>
      </section>
      <section class="panel worker-pool-panel">
        <div class="panel-head worker-pool-head"><div><h2>实例与并行生成</h2><p>一台实例同时处理一路视频；生成时由用户选择排队、智能安排或加速并行。</p></div><div><button class="btn" type="button" :disabled="busy" @click="refreshCompute"><RefreshCw :size="16"/>刷新</button><button class="btn primary" type="button" :disabled="busy || !elasticTemplateAvailable" @click="openElasticCreate"><Plus :size="16"/>新增弹性实例</button></div></div>
        <div v-if="computeNotice" class="worker-notice" :class="computeNoticeTone">{{ computeNotice }}</div>
        <div class="instance-picker managed-picker worker-page-list">
          <article v-for="item in managedInstances" :key="item.instanceId" :class="{ selected: item.instanceId === computeConfiguration?.boundInstanceId, uncertain: item.lifecycleState === 'unknown' }">
            <div class="instance-row-main">
              <span class="instance-role" :class="item.ownership === 'zhihua_managed' ? 'managed' : ''">{{ roleLabel(item.role) }}</span>
              <div><b>{{ item.name ?? item.instanceId }}</b><small>{{ item.zone }} · {{ item.gpuType ? `RTX ${item.gpuType}` : 'GPU 规格待查询' }} · {{ managementLabel(item) }}　<em class="worker-readiness" :class="workerReadinessTone(item.instanceId)">● {{ workerReadinessLabel(item.instanceId) }}</em></small></div>
              <strong>{{ managedModeLabel(item) }}<small v-if="item.currentJobId">任务执行中</small><small v-else-if="item.instancePrice != null">¥{{ item.instancePrice }}/小时</small></strong>
            </div>
            <div class="instance-row-foot"><span>{{ formatReleaseTime(item.releaseTime) }}</span><span v-if="item.missingSince" class="danger-text">平台暂未返回，等待对账</span><div><button v-if="item.ownership === 'user_managed' && item.role === 'user_managed'" class="mini-btn create" type="button" :disabled="busy || Boolean(item.missingSince) || ['unknown', 'terminating', 'terminated', 'error'].includes(item.lifecycleState)" @click="setUserWorkerEnabled(item, true)">加入批量算力</button><button v-if="item.ownership === 'user_managed' && (item.role === 'elastic' || item.role === 'test')" class="mini-btn" type="button" :disabled="busy || item.runningMode !== 'stopped' || Boolean(item.currentJobId)" @click="setUserWorkerEnabled(item, false)">移出算力池</button><button v-if="item.role !== 'user_managed' && workerReadiness[item.instanceId]?.state !== 'ready'" class="mini-btn create" type="button" :disabled="busy || item.lifecycleState === 'unknown'" @click="provisionManagedWorkerConnection(item)">配置连接</button><button v-if="item.role !== 'user_managed' && item.runningMode !== 'gpu'" class="mini-btn" type="button" :disabled="busy || item.lifecycleState === 'unknown'" @click="prepareManagedWorker(item)">准备 GPU</button><button v-if="item.runningMode === 'gpu' || item.runningMode === 'no_gpu'" class="mini-btn" type="button" :disabled="busy || item.lifecycleState === 'unknown'" @click="checkManagedWorker(item)">检查服务</button><button v-if="item.instanceId !== computeConfiguration?.boundInstanceId" class="mini-btn" type="button" :disabled="busy || item.lifecycleState === 'unknown'" @click="bindManagedInstance(item)">设为主实例</button><span v-else class="selected-label">当前主实例</span><button v-if="releaseEligibility[item.instanceId]?.allowed" class="mini-btn danger" type="button" :disabled="busy" @click="releaseManagedInstance(item)">释放</button></div></div>
          </article>
          <p v-if="!managedInstances.length" class="empty-instances">当前没有已管理实例。先配置优云智算账户并选择一台主实例，之后可按库存创建多台弹性 worker。</p>
        </div>
        <footer class="worker-pool-foot"><ShieldCheck :size="20"/><span><b>费用保护对每台实例独立生效</b><small>客户端退出、任务异常和平台对账均不会把“未知状态”当作已关机。</small></span><button class="btn" type="button" @click="activeSection='overview'">调整关闭策略</button></footer>
      </section>
    </div>

    <div v-else-if="activeSection === 'connection'" class="connection-page">
      <section class="panel environment"><div class="panel-title"><h2>主实例环境检查</h2><button class="btn link" type="button" :disabled="busy || (!connectionInfo?.configured && !tunnelStatus.configured)" @click="refreshConnection"><RefreshCw :size="15"/>连接并检查</button></div><div class="check-list"><p v-for="(item,index) in checks" :key="item.name"><span class="service-icon">{{ ['知','⌘','◇','▧','≋','⊞'][index] }}</span>{{ item.name }}<span :class="item.tone === 'success' ? 'success-text' : 'waiting-text'"><span class="dot" :class="{ gray: item.tone !== 'success' }"></span>{{ item.state }}</span></p></div><div class="disk"><HardDrive :size="22"/><div><b>远端磁盘</b><span>服务就绪后读取系统盘与工作目录空间</span></div><strong>待检测</strong></div></section>
      <section class="panel worker-health-panel"><div class="panel-head"><div><h2>全部 worker 连接状态</h2><p>异常实例不会领取新任务，其他实例可以继续工作。</p></div><button class="btn" type="button" :disabled="busy" @click="refreshCompute"><RefreshCw :size="16"/>刷新状态</button></div><div class="worker-health-list"><article v-for="item in managedInstances" :key="item.instanceId"><span class="dot" :class="{ gray: workerReadiness[item.instanceId]?.state !== 'ready' }"></span><div><b>{{ item.name ?? item.instanceId }}</b><small>{{ item.zone }} · {{ managedModeLabel(item) }}</small></div><strong :class="workerReadinessTone(item.instanceId)">{{ workerReadinessLabel(item.instanceId) }}</strong><button class="mini-btn" type="button" :disabled="busy || item.runningMode === 'stopped'" @click="checkManagedWorker(item)">检查</button></article><p v-if="!managedInstances.length" class="empty-instances">配置算力账户后，这里会逐台显示连接和服务版本。</p></div></section>
      <section class="panel connection connection-full"><div class="panel-head"><h2>连接信息</h2><span :class="connected ? 'success-text' : 'waiting-text'"><span class="dot" :class="{ gray: !connected }"></span>{{ connectionLabel }}</span></div><div class="connection-cards"><article role="button" tabindex="0" @click="llmOpen=true" @keydown.enter="llmOpen=true"><KeyRound :size="31"/><div><b>大模型内容规划</b><span>{{ llmConfiguration?.credentialStored ? `${llmConfiguration.providerLabel} · ${llmConfiguration.model}` : '等待配置 API Key' }}</span></div><strong>›</strong></article><article role="button" tabindex="0" @click="refreshConnection" @keydown.enter="refreshConnection"><KeyRound :size="31"/><div><b>自动安全连接</b><span>{{ tunnelStatus.configured ? 'SSH 私钥已保存到 Windows 凭据库' : '等待实例连接配置' }}</span></div><strong>›</strong></article><article role="button" tabindex="0" @click="refreshConnection" @keydown.enter="refreshConnection"><Server :size="31"/><div><b>本机服务入口</b><span>{{ tunnelStatus.localUrl ?? connectionInfo?.baseUrl ?? '启动时自动分配' }}</span></div><strong>›</strong></article><article role="button" tabindex="0" @click="refreshConnection" @keydown.enter="refreshConnection"><ShieldCheck :size="31"/><div><b>版本握手　<span :class="connected ? 'success-text' : 'waiting-text'">● {{ connectionLabel }}</span></b><span>{{ connected ? `API ${probe?.apiVersion} · 服务 ${probe?.serviceVersion}` : connectionError || '点击建立连接并检查兼容性' }}</span></div><strong>›</strong></article></div></section>
    </div>

    <div v-else class="appearance-grid">
      <section class="panel theme-panel">
        <div class="panel-head"><div><h2>界面主题</h2><p>切换后立即应用到全部页面，并保存在本机。</p></div><span class="theme-current"><Palette :size="18"/>当前显示：{{ effectiveTheme === 'dark' ? '深色' : '浅色' }}</span></div>
        <div class="theme-options"><button v-for="option in themeOptions" :key="option.value" type="button" :class="{ selected: themePreference === option.value }" @click="setThemePreference(option.value)"><span class="theme-icon"><component :is="option.icon" :size="28"/></span><span><b>{{ option.label }}</b><small>{{ option.description }}</small></span><i>{{ themePreference === option.value ? '✓' : '' }}</i></button></div>
        <div class="theme-preview" :class="`preview-${effectiveTheme}`"><div class="preview-nav"><span></span><i></i><i></i><i></i></div><div class="preview-main"><header></header><section><article></article><article></article><article></article></section></div></div>
      </section>
      <section class="panel local-storage-panel">
        <div class="panel-head"><div><h2>本地存储</h2><p>可指定以后新建项目的位置，既有项目不会被静默搬移。</p></div><HardDrive :size="24"/></div>
        <div class="storage-path-card"><span><FolderOpen :size="25"/></span><div><b>当前项目根目录</b><code>{{ storageInfo?.projectsRoot ?? '正在读取…' }}</code></div></div>
        <div v-if="storageInfo?.restartRequired" class="storage-path-card pending" :class="{ unavailable: !storageInfo.configuredRootAvailable }"><span><RefreshCw :size="25"/></span><div><b>{{ storageInfo.configuredRootAvailable ? '重启后使用的新目录' : '自定义目录当前不可用' }}</b><code>{{ storageInfo.configuredProjectsRoot }}</code></div></div>
        <div class="storage-actions"><button class="btn primary" type="button" @click="chooseProjectsRoot"><FolderOpen :size="18"/>更改新项目位置</button><button class="btn" type="button" :disabled="!storageInfo || storageInfo.configuredProjectsRoot === storageInfo.defaultProjectsRoot" @click="resetProjectsRoot">恢复默认</button></div>
        <div class="storage-path-card database-card"><span><Database :size="25"/></span><div><b>项目索引数据库</b><code>{{ storageInfo?.databasePath ?? '正在读取…' }}</code></div></div>
        <div class="storage-meta"><span>数据库结构版本</span><b>v{{ storageInfo?.schemaVersion ?? '--' }}</b></div>
        <p v-if="storageNotice" class="storage-notice">{{ storageNotice }}</p>
        <button class="btn open-storage" type="button" :disabled="!storageInfo" @click="openProjectsRoot"><FolderOpen :size="19"/>打开当前项目目录</button>
      </section>
      <section class="panel appearance-note"><Palette :size="26"/><div><h2>存储边界清晰可控</h2><p>数据库继续保存在系统应用目录；素材、候选视频和导出临时文件跟随各自项目。修改位置只影响重启后创建的新项目。</p></div></section>
    </div>
    <div v-if="computeOpen" class="connection-backdrop" role="presentation" @click.self="computeOpen=false">
      <section class="connection-dialog compute-dialog" role="dialog" aria-modal="true" aria-labelledby="compute-title">
        <header><div><h2 id="compute-title">优云智算实例中心</h2><p>已有实例可授权知画自动启停并参与批量生成；用户创建的实例始终保留，不会被知画释放。</p></div><button type="button" aria-label="关闭" @click="computeOpen=false"><X :size="20"/></button></header>
        <div class="connection-form">
          <template v-if="!computeConfigured">
            <label><span>API 公钥</span><input v-model="computeForm.publicKey" autocomplete="off" placeholder="输入 PublicKey"/></label>
            <label><span>API 私钥</span><input v-model="computeForm.privateKey" type="password" autocomplete="new-password" placeholder="输入 PrivateKey"/></label>
          </template>
          <template v-else>
            <p class="compute-summary">账户已连接　·　可用余额 <b>{{ computeBalance?.amountAvailable ? `¥ ${computeBalance.amountAvailable}` : '--' }}</b></p>
            <div class="instance-center-head"><span>共 {{ managedInstances.length }} 个实例</span><div><small>主实例配置作为扩容模板</small><button class="mini-btn create" type="button" :disabled="busy || !elasticTemplateAvailable" @click="openElasticCreate"><Plus :size="14"/>新增弹性实例</button></div></div>
            <div class="instance-picker managed-picker">
              <article v-for="item in managedInstances" :key="item.instanceId" :class="{ selected: item.instanceId === computeConfiguration?.boundInstanceId, uncertain: item.lifecycleState === 'unknown' }">
                <div class="instance-row-main">
                  <span class="instance-role" :class="item.ownership === 'zhihua_managed' ? 'managed' : ''">{{ roleLabel(item.role) }}</span>
                  <div><b>{{ item.name ?? item.instanceId }}</b><small>{{ item.zone }} · {{ item.gpuType ? `RTX ${item.gpuType}` : 'GPU 规格待查询' }} · {{ managementLabel(item) }}　<em class="worker-readiness" :class="workerReadinessTone(item.instanceId)">● {{ workerReadinessLabel(item.instanceId) }}</em></small></div>
                  <strong>{{ managedModeLabel(item) }}</strong>
                </div>
                <div class="instance-row-foot"><span>{{ formatReleaseTime(item.releaseTime) }}</span><span v-if="item.missingSince" class="danger-text">平台暂未返回，等待对账</span><div><button v-if="item.ownership === 'user_managed' && item.role === 'user_managed'" class="mini-btn create" type="button" :disabled="busy || Boolean(item.missingSince) || ['unknown', 'terminating', 'terminated', 'error'].includes(item.lifecycleState)" @click="setUserWorkerEnabled(item, true)">加入批量算力</button><button v-if="item.ownership === 'user_managed' && (item.role === 'elastic' || item.role === 'test')" class="mini-btn" type="button" :disabled="busy || item.runningMode !== 'stopped' || Boolean(item.currentJobId)" title="关机且没有任务时才能移出" @click="setUserWorkerEnabled(item, false)">移出批量算力</button><button v-if="item.role !== 'user_managed' && workerReadiness[item.instanceId]?.state !== 'ready'" class="mini-btn create" type="button" :disabled="busy || item.lifecycleState === 'unknown'" title="已关机时仅无卡启动，不产生 GPU 费用" @click="provisionManagedWorkerConnection(item)">配置连接</button><button v-if="item.role !== 'user_managed' && item.runningMode !== 'gpu'" class="mini-btn" type="button" :disabled="busy || item.lifecycleState === 'unknown'" @click="prepareManagedWorker(item)">准备 GPU</button><button v-if="item.runningMode === 'gpu' || item.runningMode === 'no_gpu'" class="mini-btn" type="button" :disabled="busy || item.lifecycleState === 'unknown'" @click="checkManagedWorker(item)">检查服务</button><button v-if="item.instanceId !== computeConfiguration?.boundInstanceId" class="mini-btn" type="button" :disabled="busy || item.lifecycleState === 'unknown'" @click="bindManagedInstance(item)">设为主实例</button><span v-else class="selected-label">当前主实例</span><button v-if="releaseEligibility[item.instanceId]?.allowed" class="mini-btn danger" type="button" :disabled="busy" @click="releaseManagedInstance(item)">释放</button></div></div>
              </article>
              <p v-if="!managedInstances.length" class="empty-instances">平台没有返回可用实例。创建弹性实例前会先检查地域库存和实时报价，并再次让你确认。</p>
            </div>
          </template>
          <p v-if="computeNotice" class="connection-notice" :class="computeNoticeTone">{{ computeNotice }}</p>
        </div>
        <footer><span></span><span></span><button class="btn" type="button" :disabled="busy" @click="refreshCompute">刷新</button><button v-if="!computeConfigured" class="btn primary" type="button" :disabled="busy" @click="saveComputeCredentials">保存并验证</button><button v-else class="btn primary" type="button" @click="computeOpen=false">完成</button></footer>
      </section>
    </div>

    <div v-if="elasticCreateOpen" class="connection-backdrop" role="presentation" @click.self="closeElasticCreate">
      <section class="connection-dialog elastic-create-dialog" role="dialog" aria-modal="true" aria-labelledby="elastic-create-title">
        <header><div><h2 id="elastic-create-title">创建弹性实例</h2><p>复用当前主实例的地域、镜像和单卡规格。确认后创建按量实例；全部结果完成本地校验后自动关机并释放实例，数据盘默认保留。</p></div><button type="button" aria-label="关闭" :disabled="elasticCreateBusy" @click="closeElasticCreate"><X :size="20"/></button></header>
        <div class="connection-form elastic-create-form">
          <label><span>实例名称</span><input v-model.trim="elasticForm.name" maxlength="63" autocomplete="off"/></label>
          <label><span>创建数量</span><input v-model.number="elasticForm.count" type="number" min="1" max="50" step="1"/><small>每台实例分别使用幂等操作；实时库存和账号配额不足时允许部分成功，知画不会自动补购。</small></label>
          <div class="elastic-summary-grid">
            <article><span>地域 / 可用区</span><b>{{ elasticForm.region }} / {{ elasticForm.zone }}</b></article>
            <article><span>GPU</span><b>1 × RTX {{ elasticForm.gpuType }}</b></article>
            <article><span>CPU / 内存</span><b>{{ elasticForm.cpu }} 核 / {{ elasticForm.memoryGb }} GB</b></article>
            <article><span>数量 / 计费</span><b>{{ elasticCreateCount }} 台 · 按量后付费</b></article>
          </div>
          <label><span>镜像 ID</span><input v-model.trim="elasticForm.imageId" autocomplete="off"/><small>弹性 worker 必须使用已经包含知画服务的兼容镜像；创建后仍会执行版本与模型清单检查。</small></label>
          <div v-if="elasticPreflight" class="preflight-result" :class="{ unavailable: !elasticPreflight.capacityAvailable }">
            <span>{{ elasticPreflight.capacityAvailable ? '单台规格当前有库存' : '当前无匹配库存' }}</span>
            <b>{{ elasticTotalHourlyPrice == null ? '平台未返回费率' : `合计约 ¥ ${elasticTotalHourlyPrice.toFixed(2)} / 小时` }}</b>
            <small>单台约 {{ elasticPreflight.estimatedHourlyPrice == null ? '--' : `¥ ${elasticPreflight.estimatedHourlyPrice.toFixed(2)} / 小时` }}；查询时间 {{ new Date(elasticPreflight.checkedAt).toLocaleString('zh-CN') }}。多台库存以各创建响应和实际账单为准。</small>
          </div>
          <p v-if="elasticCreateNotice" class="connection-notice elastic-notice" :class="elasticCreateNoticeTone">{{ elasticCreateNotice }}</p>
        </div>
        <footer><span></span><span></span><button class="btn" type="button" :disabled="elasticCreateBusy" @click="closeElasticCreate">取消</button><button v-if="!elasticPreflight?.capacityAvailable" class="btn primary" type="button" :disabled="elasticCreateBusy || !elasticForm.name || !elasticForm.imageId || elasticCreateCount < 1" @click="preflightElasticCreate"><RefreshCw :size="16"/>{{ elasticCreateBusy ? '正在查询' : '查询库存与报价' }}</button><button v-else class="btn primary" type="button" :disabled="elasticCreateBusy" @click="confirmElasticCreate"><Plus :size="16"/>{{ elasticCreateBusy ? '正在创建' : `确认创建 ${elasticCreateCount} 台并开始计费` }}</button></footer>
      </section>
    </div>

    <div v-if="llmOpen" class="connection-backdrop" role="presentation" @click.self="llmOpen=false">
      <section class="connection-dialog" role="dialog" aria-modal="true" aria-labelledby="llm-title">
        <header><div><h2 id="llm-title">大模型内容规划</h2><p>资料只会在你点击提取或修订时发送；每个提供商的 API Key 分开保存在 Windows 凭据管理器。</p></div><button type="button" aria-label="关闭" @click="llmOpen=false"><X :size="20"/></button></header>
        <div class="connection-form">
          <label><span>提供商</span><select :value="llmForm.providerId" @change="selectLlmProvider"><option v-for="preset in llmProviderPresets" :key="preset.id" :value="preset.id">{{ preset.label }}</option></select></label>
          <label><span>API 地址</span><input v-model="llmForm.baseUrl" autocomplete="off"/></label>
          <label><span>模型 / 接入点</span><input v-model="llmForm.model" autocomplete="off" :placeholder="selectedLlmPreset.modelHint"/></label>
          <label><span>API Key</span><input v-model="llmForm.apiKey" type="password" autocomplete="new-password" :placeholder="llmConfigured ? '已保存；留空可只测试连接' : `输入 ${selectedLlmPreset.label} API Key`"/></label>
          <p v-if="llmNotice" class="connection-notice" :class="llmNoticeTone">{{ llmNotice }}</p>
        </div>
        <footer><button v-if="llmConfigured" class="btn link danger" type="button" :disabled="busy" @click="clearLlm">移除密钥</button><span></span><button class="btn" type="button" @click="llmOpen=false">取消</button><button class="btn primary" type="button" :disabled="busy" @click="saveOrTestLlm">{{ llmConfigured && !llmForm.apiKey ? '保存并测试' : '保存并验证' }}</button></footer>
      </section>
    </div>

  </section>
</template>

<script lang="ts">
import { FileText as FileTextIcon } from "lucide-vue-next";
export default { components: { FileTextIcon } };
</script>

<style scoped>
.settings-page{display:grid;grid-template-rows:78px 61px minmax(0,1fr);gap:0}.settings-head{display:flex;align-items:center;padding-left:16px}.settings-tabs{display:flex;gap:30px;border-bottom:1px solid var(--line);padding:0 20px}.settings-tabs button{height:61px;padding:0 12px;border:0;background:transparent;display:flex;align-items:center;gap:12px;font-size:17px;font-weight:700;color:#29446f;position:relative}.settings-tabs button.active{color:var(--blue)}.settings-tabs button.active:after{content:"";position:absolute;left:0;right:0;bottom:0;height:4px;border-radius:4px;background:var(--blue)}.settings-grid{min-height:0;padding-top:14px;display:grid;grid-template-columns:1.05fr 1fr 1fr;grid-template-rows:294px minmax(288px,1fr) 160px;gap:13px}.account{grid-column:1/3}.instance{grid-column:3/4}.connection{grid-column:1/4}.panel-head>span,.panel-title>span{display:flex;align-items:center;gap:8px;font-size:14px}.money-row{height:126px;margin:10px 18px 12px;display:grid;grid-template-columns:1fr .85fr 1.05fr;border-radius:9px;overflow:hidden;background:linear-gradient(110deg,#eaf3ff,#f7fbff)}.money-row article{display:flex;align-items:center;gap:16px;padding:16px 20px;border-right:1px solid #d7e2f0}.money-row svg{color:var(--blue)}.money-row article>div,.money-row article{color:#1b3159}.money-row span{display:block;font-size:14px}.money-row b{display:block;margin-top:8px;font-size:31px;color:#07183d}.money-row b small{font-size:14px}.money-row .cost{background:linear-gradient(120deg,#fff8ef,#fffaf5);color:#263d64}.money-row .cost svg,.money-row .cost b{color:#f26b0f}.money-row .cost small{display:block;margin-top:5px;color:#677b9d;font-size:14px}.account-actions{display:grid;grid-template-columns:1fr 1fr;gap:14px;padding:0 18px}.account-actions .btn{height:50px;font-size:17px}.instance-main{height:105px;display:grid;grid-template-columns:104px 1fr auto;align-items:center;padding:8px 18px;gap:14px}.server-art{width:100px;height:85px;display:grid;place-items:center;background:linear-gradient(145deg,#d7e7f7,#f8fbff);border-radius:8px;font-size:62px;color:#274b73}.instance-main h2{font-size:20px}.instance-main p{font-size:17px;margin:5px 0}.instance-main span{display:flex;align-items:center;gap:7px;font-size:14px}.instance-main>span{align-self:center}.instance-metrics{height:66px;display:grid;grid-template-columns:.8fr 1.25fr 1fr;border-top:1px solid #e1e8f1;padding:8px 16px}.instance-metrics>div{display:flex;flex-direction:column;border-right:1px solid #dfe6ef;padding-left:8px}.instance-metrics>div:last-child{border:0}.instance-metrics span{font-size:14px;color:#617596}.instance-metrics b{font-size:16px;margin-top:5px}.instance-actions{display:grid;grid-template-columns:.8fr 1fr 1.55fr;gap:9px;padding:6px 18px}.instance-actions .btn{min-height:40px;padding:0 8px}.panel-title{height:55px;padding:0 21px;display:flex;align-items:center;justify-content:space-between}.switch{width:49px;height:27px;border-radius:20px;background:#cbd6e6;position:relative}.switch:after{content:"";position:absolute;left:3px;top:3px;width:21px;height:21px;border-radius:50%;background:white;box-shadow:0 1px 4px #9aabc2}.switch.on{background:var(--blue)}.switch.on:after{left:25px}.setting-row{min-height:74px;margin:0 22px;border-bottom:1px solid #e0e7f1;display:flex;align-items:center;gap:15px}.setting-row svg{color:#16355f}.setting-row>div{display:flex;flex-direction:column;gap:6px;flex:1}.setting-row b{font-size:15px}.setting-row span{font-size:14px;color:#687c9e}.setting-row button{height:43px;min-width:150px;border:1px solid #cddbee;border-radius:7px;background:#fff}.check-list{padding:0 18px}.check-list p{height:33px;display:flex;align-items:center;gap:12px;border-bottom:1px solid #e7edf4;font-size:14px}.check-list p>span:last-child{margin-left:auto;display:flex;align-items:center;gap:7px}.service-icon{width:23px;height:23px;display:grid;place-items:center;color:#19407c;font-weight:800}.disk{height:57px;display:grid;grid-template-columns:30px 92px 1fr 120px 36px;align-items:center;gap:8px;padding:0 18px}.disk>span{font-size:14px;color:#5f7394}.warning-box{margin:12px 17px;padding:15px;border-radius:8px;background:#fff3e4;color:#ed6a0d;display:flex;flex-direction:column;gap:6px}.warning-box span{font-size:14px;color:#745d4e}.connection-cards{height:102px;display:grid;grid-template-columns:repeat(4,1fr);gap:10px;padding:12px 18px}.connection-cards article{border:1px solid #dbe4f0;border-radius:8px;display:grid;grid-template-columns:42px 1fr 15px;align-items:center;padding:0 13px;min-width:0}.connection-cards svg{color:var(--blue)}.connection-cards article>div{display:flex;flex-direction:column;gap:6px;min-width:0}.connection-cards span{font-size:14px;color:#687c9e;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.connection-cards .success-text{font-size:14px}.retention .setting-row{min-height:70px}@media(max-width:1380px){.settings-grid{grid-template-rows:270px minmax(260px,1fr) 145px}.settings-tabs{gap:14px}.money-row{height:112px}.money-row b{font-size:25px}.instance-main{height:92px}.server-art{width:80px;height:70px}.instance-actions{padding-left:10px;padding-right:10px}.connection-cards{height:88px}.connection-cards article{grid-template-columns:34px 1fr 12px;padding:0 9px}}
.waiting-text{color:#6e809d}
.appearance-grid{min-height:0;padding-top:14px;display:grid;grid-template-columns:minmax(640px,1.45fr) minmax(440px,1fr);grid-template-rows:minmax(0,1fr) 92px;gap:13px}.theme-panel,.local-storage-panel{overflow:hidden}.theme-panel>.panel-head,.local-storage-panel>.panel-head{min-height:74px}.theme-panel>.panel-head>div p,.local-storage-panel>.panel-head>div p{margin-top:5px;color:var(--muted);font-size:14px}.theme-current{display:flex;align-items:center;gap:7px;color:var(--blue);font-size:14px;font-weight:700}.theme-options{padding:18px;display:grid;grid-template-columns:repeat(3,1fr);gap:12px}.theme-options>button{min-height:96px;padding:14px;border:1px solid var(--line);border-radius:10px;background:var(--surface);display:grid;grid-template-columns:47px 1fr 20px;align-items:center;gap:10px;text-align:left}.theme-options>button:hover{border-color:#94baf1}.theme-options>button.selected{border-color:var(--blue);box-shadow:0 0 0 1px var(--blue) inset;background:var(--blue-soft)}.theme-icon{width:44px;height:44px;border-radius:10px;display:grid;place-items:center;color:var(--blue);background:var(--blue-soft)}.theme-options b,.theme-options small{display:block}.theme-options b{font-size:17px}.theme-options small{margin-top:6px;color:var(--muted);font-size:14px;line-height:1.4}.theme-options i{font-style:normal;color:var(--blue);font-weight:800}.theme-preview{height:calc(100% - 206px);min-height:210px;margin:0 18px 18px;border:1px solid var(--line);border-radius:12px;overflow:hidden;display:grid;grid-template-columns:112px 1fr;box-shadow:0 12px 30px rgba(25,62,111,.12)}.theme-preview.preview-light{background:#f4f8fe}.theme-preview.preview-dark{background:#101a2b;border-color:#30415a}.preview-nav{padding:18px 14px;display:flex;flex-direction:column;gap:12px;background:rgba(255,255,255,.9);border-right:1px solid #dce6f3}.preview-dark .preview-nav{background:#111d2f;border-color:#2b3c55}.preview-nav span{height:28px;border-radius:7px;background:var(--blue)}.preview-nav i{height:15px;border-radius:5px;background:#dfe8f5}.preview-dark .preview-nav i{background:#293951}.preview-main{padding:18px}.preview-main header{height:32px;width:45%;margin-bottom:15px;border-radius:6px;background:#d9e5f5}.preview-dark .preview-main header{background:#293b56}.preview-main section{display:grid;grid-template-columns:repeat(3,1fr);gap:12px}.preview-main article{height:115px;border:1px solid #d8e3f1;border-radius:9px;background:#fff}.preview-dark .preview-main article{border-color:#30415a;background:#18263a}.local-storage-panel{padding-bottom:18px}.local-storage-panel>.panel-head svg{color:var(--blue)}.storage-path-card{min-height:91px;margin:14px 18px 0;padding:13px;border:1px solid var(--line);border-radius:9px;display:grid;grid-template-columns:46px minmax(0,1fr);align-items:center;gap:12px;background:var(--surface-soft)}.storage-path-card>span{width:43px;height:43px;border-radius:9px;display:grid;place-items:center;color:var(--blue);background:var(--blue-soft)}.storage-path-card b,.storage-path-card code{display:block}.storage-path-card code{margin-top:7px;color:var(--muted);font:14px/1.4 Consolas,monospace;user-select:text;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.storage-meta{height:50px;margin:14px 18px 0;padding:0 14px;border:1px solid var(--line);border-radius:8px;display:flex;align-items:center;justify-content:space-between}.storage-meta span{color:var(--muted)}.storage-notice{margin:10px 18px;color:#c5483e;font-size:14px}.open-storage{width:calc(100% - 36px);height:48px;margin:14px 18px 0}.appearance-note{grid-column:1/3;display:flex;align-items:center;gap:15px;padding:0 22px}.appearance-note>svg{color:var(--blue)}.appearance-note p{margin-top:5px;color:var(--muted);font-size:14px}@media(max-width:1380px){.appearance-grid{grid-template-columns:1.35fr 1fr}.theme-options{padding:13px}.theme-options>button{padding:10px;grid-template-columns:40px 1fr 16px}.theme-icon{width:38px;height:38px}.theme-preview{height:calc(100% - 190px);margin:0 13px 13px}.storage-path-card{margin-left:13px;margin-right:13px}.open-storage{width:calc(100% - 26px);margin-left:13px;margin-right:13px}}
.settings-tabs button:disabled{opacity:.48;cursor:not-allowed}.connection-cards article[role="button"]{cursor:pointer;transition:.15s ease}.connection-cards article[role="button"]:hover,.connection-cards article[role="button"]:focus-visible{border-color:#8eb8f7;background:#f7faff;outline:0}.connection-backdrop{position:fixed;z-index:80;inset:60px 0 0 0;background:rgba(6,20,46,.32);display:grid;place-items:center;padding:24px}.connection-dialog{width:min(620px,calc(100vw - 80px));border:1px solid #cedbed;border-radius:13px;background:#fff;box-shadow:0 24px 70px rgba(16,45,88,.24);overflow:hidden}.connection-dialog>header{min-height:88px;padding:19px 22px;display:flex;align-items:flex-start;justify-content:space-between;border-bottom:1px solid var(--line);background:linear-gradient(135deg,#f7fbff,#fff)}.connection-dialog>header h2{font-size:22px}.connection-dialog>header p{margin-top:7px;color:#64799b;font-size:14px}.connection-dialog>header button{width:34px;height:34px;border:0;border-radius:7px;background:transparent;display:grid;place-items:center}.connection-dialog>header button:hover{background:#edf3fb}.connection-form{padding:20px 22px;display:flex;flex-direction:column;gap:16px}.connection-form label{display:grid;grid-template-columns:132px minmax(0,1fr);align-items:center;gap:8px 14px}.connection-form label>span{font-weight:700;color:#1b355f}.connection-form input,.connection-form select{height:43px;border:1px solid #cbd9ec;border-radius:8px;padding:0 12px;background:#fff;user-select:text}.connection-form input:focus,.connection-form select:focus{outline:2px solid #cfe2ff;border-color:var(--blue)}.connection-form small{grid-column:2;color:#6d809f;font-size:14px}.connection-notice{margin:2px 0 0 146px;padding:10px 12px;border-radius:7px;background:#f1f5fa;color:#52698e;font-size:14px}.connection-notice.success{background:#e8f8f1;color:#087d51}.connection-notice.error{background:#fff0ee;color:#b33b35}.connection-dialog>footer{min-height:72px;padding:13px 22px;border-top:1px solid var(--line);display:grid;grid-template-columns:auto 1fr auto auto;align-items:center;gap:10px;background:#fbfdff}.connection-dialog button:disabled,.environment button:disabled{opacity:.55;cursor:not-allowed}.compute-summary{margin:0;padding:13px 15px;border-radius:8px;background:#eef6ff;color:#29486f}.compute-summary b{color:var(--blue);font-size:20px}.instance-picker{display:flex;flex-direction:column;gap:9px;max-height:280px;overflow:auto}.instance-picker>button{min-height:68px;padding:10px 14px;border:1px solid #d7e2ef;border-radius:8px;background:#fff;display:flex;align-items:center;justify-content:space-between;text-align:left;color:#17345f}.instance-picker>button.selected{border-color:var(--blue);background:#f1f7ff;box-shadow:0 0 0 1px var(--blue) inset}.instance-picker>button span{display:flex;flex-direction:column;gap:6px}.instance-picker>button small{color:#7183a0}.instance-picker>button strong{color:#315d9b;font-size:14px}
.settings-tabs-spacer{flex:1}.policy-badge{padding:5px 9px;border-radius:14px;background:#e8f8f1;color:#087d51;font-size:14px;font-weight:700}.policy-badge.neutral{background:var(--surface-soft);color:var(--muted)}
.policy-options{padding:0 14px;display:grid;gap:7px}.policy-options>button{min-height:55px;padding:7px 30px 7px 10px;border:1px solid #dbe4f0;border-radius:8px;background:#f9fbfe;text-align:left;position:relative;color:var(--text)}.policy-options>button.selected{border-color:var(--blue);background:#edf5ff;box-shadow:0 0 0 1px rgba(15,105,255,.08)}.policy-options>button>span{display:flex;align-items:center;justify-content:space-between;gap:8px}.policy-options b{font-size:14px}.policy-options small{color:var(--blue);font-size:14px;font-weight:700}.policy-options p{margin-top:3px;color:#657a9a;font-size:14px;line-height:1.3}.policy-options i{position:absolute;right:10px;top:19px;width:18px;height:18px;border-radius:50%;display:grid;place-items:center;background:var(--blue);color:white;font-style:normal}.policy-guard{height:48px;margin:7px 14px 0;padding-top:7px;border-top:1px solid #e1e8f1;display:grid;grid-template-columns:27px minmax(0,1fr) 72px;align-items:center;gap:5px}.policy-guard>svg{color:#16355f}.policy-guard>span{min-width:0;display:flex;flex-direction:column}.policy-guard b{font-size:14px}.policy-guard small{color:#657a9a;font-size:14px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.policy-guard button{height:30px;border:1px solid #cfdbea;border-radius:6px;background:#fff;color:#31537f;font-size:14px}
.compute-dialog{width:min(790px,calc(100vw - 80px))}.compute-dialog .connection-form{max-height:min(610px,calc(100vh - 250px));overflow:auto}.instance-center-head{display:flex;align-items:center;justify-content:space-between;color:#23446f;font-weight:700}.instance-center-head>div{display:flex;align-items:center;gap:9px}.instance-center-head small{color:#7183a0;font-weight:400}.instance-center-head .create{display:flex;align-items:center;gap:4px;border-color:#8db7f2;color:var(--blue)}.managed-picker{max-height:390px}.managed-picker>article{padding:13px 14px;border:1px solid #d7e2ef;border-radius:9px;background:#fff}.managed-picker>article.selected{border-color:var(--blue);background:#f4f8ff;box-shadow:0 0 0 1px var(--blue) inset}.managed-picker>article.uncertain{border-style:dashed;background:#fafbfc}.instance-row-main{display:grid;grid-template-columns:72px minmax(0,1fr) auto;align-items:center;gap:12px}.instance-row-main>div{min-width:0}.instance-row-main b,.instance-row-main small{display:block}.instance-row-main b{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#17345f}.instance-row-main small{margin-top:5px;color:#7183a0;font-size:14px}.instance-row-main strong{color:#315d9b;font-size:14px}.instance-role{padding:5px 7px;border-radius:13px;background:#edf1f6;color:#5e708d;text-align:center;font-size:14px;font-weight:700}.instance-role.managed{background:#e9f8f1;color:#087d51}.instance-row-foot{margin-top:10px;padding-top:9px;border-top:1px solid #e6edf5;display:flex;align-items:center;gap:12px;color:#7183a0;font-size:14px}.instance-row-foot>div{margin-left:auto;display:flex;align-items:center;gap:8px}.selected-label{color:var(--blue);font-weight:700}.mini-btn{height:29px;padding:0 10px;border:1px solid #b9cce5;border-radius:6px;background:#fff;color:#285184;font-size:14px}.mini-btn.danger{border-color:#edb4ad;color:#b83830}.danger-text{color:#bd463d}.empty-instances{padding:24px 18px;border:1px dashed #cbd9eb;border-radius:9px;color:#6a7f9f;line-height:1.6;text-align:center}
.elastic-create-dialog{width:min(720px,calc(100vw - 80px))}.elastic-create-form{gap:13px}.elastic-summary-grid{display:grid;grid-template-columns:1fr 1fr;gap:9px}.elastic-summary-grid article{min-height:68px;padding:11px 13px;border:1px solid var(--line);border-radius:8px;background:var(--surface-soft)}.elastic-summary-grid span,.elastic-summary-grid b{display:block}.elastic-summary-grid span{color:var(--muted);font-size:14px}.elastic-summary-grid b{margin-top:7px;font-size:15px}.preflight-result{padding:13px 15px;border:1px solid #a8dec8;border-radius:8px;background:#effaf5;display:grid;grid-template-columns:1fr auto;gap:5px 12px;color:#087d51}.preflight-result.unavailable{border-color:#f2b5aa;background:#fff3f0;color:#b13c32}.preflight-result span{font-size:14px;font-weight:700}.preflight-result b{font-size:16px}.preflight-result small{grid-column:1/-1;color:var(--muted);font-size:14px}.elastic-notice{margin-left:0}.elastic-create-dialog footer .btn{display:flex;align-items:center;gap:6px}.money-row .mode-card{padding-left:22px}.money-row .mode-card>div{min-width:0}.money-row .mode-card b{font-size:25px;white-space:nowrap}.instance-main{grid-template-columns:76px minmax(0,1fr);padding:9px 14px;gap:12px}.server-art{width:72px;height:72px;font-size:48px}.instance-identity{min-width:0}.instance-identity h2{font-size:18px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.instance-identity p{font-size:15px;margin:5px 0}.instance-identity>span{font-size:14px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.disk{height:57px;display:grid;grid-template-columns:28px minmax(0,1fr) auto;align-items:center;gap:9px;padding:0 18px}.disk>div{min-width:0}.disk b,.disk span{display:block}.disk span{margin-top:3px;color:#6d809f;font-size:14px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.disk strong{color:#6d809f;font-size:14px}
.worker-readiness{font-style:normal}.worker-readiness.ready{color:#09875b}.worker-readiness.pending{color:#6b80a0}.worker-readiness.error{color:#c4423a}
.settings-tabs{gap:18px}.settings-tabs button>i{min-width:20px;height:20px;padding:0 6px;border-radius:10px;display:grid;place-items:center;background:var(--blue-soft);color:var(--blue);font-size:14px;font-style:normal}.settings-overview{grid-template-rows:294px minmax(0,1fr)}.settings-overview .shutdown{grid-column:1/2}.overview-retention{grid-column:2/4;position:relative;padding-bottom:62px}.overview-retention .worker-link{position:absolute;right:18px;bottom:14px;min-height:36px;font-size:14px}.worker-page{min-height:0;padding-top:14px;display:grid;grid-template-rows:98px minmax(0,1fr);gap:13px}.worker-summary{display:grid;grid-template-columns:repeat(4,1fr);gap:12px}.worker-summary article{padding:15px 18px;border:1px solid var(--line);border-radius:10px;background:var(--panel);box-shadow:var(--shadow);display:grid;grid-template-columns:1fr auto;gap:5px 12px;align-items:center}.worker-summary span{color:var(--muted);font-size:14px}.worker-summary b{grid-row:1/3;grid-column:2;font-size:29px;color:var(--navy)}.worker-summary small{font-size:14px}.worker-pool-panel{min-height:0;display:flex;flex-direction:column;overflow:hidden}.worker-pool-head{min-height:68px;padding-block:9px}.worker-pool-head>div:first-child{min-width:0}.worker-pool-head p,.worker-health-panel .panel-head p{margin-top:5px;color:var(--muted);font-size:14px}.worker-pool-head>div:last-child{display:flex;gap:8px}.worker-pool-head .btn{min-height:38px;font-size:14px}.worker-notice{margin:10px 16px 0;padding:9px 12px;border-radius:7px;background:var(--surface-soft);color:var(--muted);font-size:14px}.worker-notice.success{background:#e8f8f1;color:#087d51}.worker-notice.error{background:#fff0ee;color:#b33b35}.worker-page-list{flex:1;min-height:0;max-height:none;padding:12px 16px;overflow:auto}.worker-page-list .instance-row-main{grid-template-columns:82px minmax(0,1fr) 120px}.worker-page-list .instance-row-main>strong{display:flex;flex-direction:column;align-items:flex-end;gap:4px}.worker-page-list .instance-row-main>strong small{margin:0;font-size:14px;font-weight:400}.worker-pool-foot{min-height:64px;padding:9px 16px;border-top:1px solid var(--line);display:grid;grid-template-columns:28px minmax(0,1fr) auto;align-items:center;gap:10px}.worker-pool-foot>svg{color:var(--green)}.worker-pool-foot>span{display:flex;flex-direction:column;gap:4px}.worker-pool-foot small{color:var(--muted);font-size:14px}.worker-pool-foot .btn{min-height:36px;font-size:14px}.connection-page{min-height:0;padding-top:14px;display:grid;grid-template-columns:minmax(500px,1fr) minmax(500px,1fr);grid-template-rows:minmax(0,1fr) 150px;gap:13px}.connection-page>.environment,.worker-health-panel{min-height:0;overflow:hidden}.connection-full{grid-column:1/3}.worker-health-panel{display:flex;flex-direction:column}.worker-health-panel>.panel-head{min-height:62px}.worker-health-panel>.panel-head>div{min-width:0}.worker-health-list{padding:8px 14px;overflow:auto}.worker-health-list article{min-height:62px;padding:7px 10px;border-bottom:1px solid var(--line);display:grid;grid-template-columns:10px minmax(0,1fr) 100px 62px;align-items:center;gap:10px}.worker-health-list article>div{min-width:0;display:flex;flex-direction:column;gap:4px}.worker-health-list article b,.worker-health-list article small{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.worker-health-list article strong{text-align:right;font-size:14px}.worker-health-list article strong.ready{color:var(--green)}.worker-health-list article strong.error{color:#c4423a}.storage-actions{margin:12px 18px 0;display:grid;grid-template-columns:1fr 120px;gap:9px}.storage-actions .btn{min-height:42px;font-size:14px}.storage-path-card.pending{border-color:#9bc1f5;background:var(--blue-soft)}.storage-path-card.pending.unavailable{border-color:#efb272;background:#fff7ea}.storage-path-card.pending.unavailable>span{color:#d9660a;background:#fff0db}.storage-path-card.database-card{min-height:76px}.local-storage-panel .open-storage{margin-top:10px;min-height:40px;height:40px}.local-storage-panel:has(.storage-path-card.pending) .storage-path-card{min-height:74px;margin-top:9px}.local-storage-panel:has(.storage-path-card.pending) .storage-meta{height:42px;margin-top:9px}
.settings-overview{grid-template-columns:repeat(3,minmax(0,1fr));grid-template-rows:214px minmax(0,1fr)}
.overview-metrics{grid-column:1/4;overflow:hidden}
.overview-metrics>.panel-head{min-height:58px}
.overview-metrics>.panel-head>div:first-child p{margin-top:4px;color:var(--muted);font-size:14px}
.overview-actions{display:flex;align-items:center;gap:8px}
.overview-stat-grid{height:calc(100% - 58px);padding:14px 17px 17px;display:grid;grid-template-columns:repeat(4,1fr);gap:12px}
.overview-stat-grid article{min-width:0;padding:16px;border:1px solid var(--line);border-radius:11px;background:var(--surface-soft);display:grid;grid-template-columns:48px minmax(0,1fr);align-items:center;gap:13px}
.overview-stat-icon{width:46px;height:46px;border-radius:12px;display:grid;place-items:center;color:var(--blue);background:var(--blue-soft)}
.overview-stat-grid small,.overview-stat-grid b,.overview-stat-grid p{display:block;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.overview-stat-grid small{color:var(--muted);font-size:14px;font-weight:700}
.overview-stat-grid b{margin-top:5px;color:var(--navy);font-size:24px;line-height:1.15}
.overview-stat-grid p{margin-top:6px;color:var(--muted);font-size:14px}
.settings-overview .instance{grid-column:1/2;display:flex;flex-direction:column}
.settings-overview .instance .instance-main{height:132px;padding-block:17px}
.settings-overview .instance .instance-metrics{height:78px;padding-block:12px}
.settings-overview .instance .instance-actions{margin-top:auto;padding-bottom:16px}
.settings-overview .shutdown{grid-column:2/3}
.settings-overview .shutdown .policy-options{gap:10px}
.settings-overview .shutdown .policy-options>button{min-height:76px;padding:11px 34px 11px 13px}
.settings-overview .shutdown .policy-options p{margin-top:5px;line-height:1.45}
.settings-overview .shutdown .policy-options i{top:28px}
.settings-overview .shutdown .policy-guard{height:64px;margin-top:10px}
.overview-retention{grid-column:3/4;padding-bottom:66px}
.lifecycle-list{padding:0 16px;display:grid;gap:9px}
.lifecycle-list article{min-height:72px;padding:11px 12px;border:1px solid var(--line);border-radius:9px;background:var(--surface-soft);display:grid;grid-template-columns:29px minmax(0,1fr);align-items:center;gap:9px}
.lifecycle-list article>svg{color:var(--blue)}
.lifecycle-list b,.lifecycle-list span{display:block}
.lifecycle-list b{font-size:14px}
.lifecycle-list span{margin-top:5px;color:var(--muted);font-size:14px;line-height:1.4}
.overview-retention .warning-box.compact{margin:10px 16px;padding:11px 13px}
.overview-retention .worker-link{position:absolute;left:16px;right:16px;bottom:14px;min-height:40px;font-size:14px}
:global(:root[data-theme="dark"]) .worker-summary article,:global(:root[data-theme="dark"]) .managed-picker>article,:global(:root[data-theme="dark"]) .mini-btn{border-color:var(--line);background:var(--surface)}:global(:root[data-theme="dark"]) .worker-summary b,:global(:root[data-theme="dark"]) .instance-row-main b{color:var(--text)}:global(:root[data-theme="dark"]) .managed-picker>article.selected{border-color:var(--blue);background:var(--blue-soft)}:global(:root[data-theme="dark"]) .worker-notice.error{color:#ff9a91;background:#3a2023}:global(:root[data-theme="dark"]) .worker-notice.success{color:#66d9a7;background:#15352d}:global(:root[data-theme="dark"]) .connection-cards article[role="button"]:hover{background:#1d2d44}
:global(:root[data-theme="dark"]) .policy-options>button{border-color:var(--line);background:var(--surface-soft);color:var(--text)}
:global(:root[data-theme="dark"]) .policy-options>button.selected{border-color:var(--blue);background:var(--blue-soft)}
:global(:root[data-theme="dark"]) .policy-options p,:global(:root[data-theme="dark"]) .policy-guard small{color:var(--muted)}
:global(:root[data-theme="dark"]) .policy-guard button{border-color:var(--line);background:var(--surface);color:var(--text)}
.mini-btn{height:34px;padding-inline:12px}
.worker-health-list article{min-height:72px}
.worker-page-list .managed-picker>article{padding-block:15px}
@media(max-width:1380px){.settings-tabs{gap:7px;padding-inline:12px}.settings-tabs button{padding-inline:8px;font-size:14px;gap:7px}.worker-page{grid-template-rows:86px minmax(0,1fr)}.worker-summary article{padding:11px 13px}.worker-summary b{font-size:24px}.connection-page{grid-template-columns:1fr 1fr}.worker-page-list{padding:9px 12px}.instance-row-foot>div{gap:5px}.mini-btn{padding-inline:7px}}
.advanced-label{align-self:center;padding-left:14px;border-left:1px solid var(--line);color:var(--muted);font-size:14px;font-weight:700;white-space:nowrap}
.service-page{min-height:0;padding-top:14px;display:grid;grid-template-rows:190px 190px minmax(130px,1fr);gap:14px}
.service-hero{padding:26px 28px;display:grid;grid-template-columns:76px minmax(320px,1fr) minmax(230px,.55fr) auto;align-items:center;gap:22px;overflow:hidden;position:relative}
.service-hero:before{content:"";position:absolute;inset:0 auto 0 0;width:5px;background:var(--blue)}
.service-hero.tone-ready:before{background:var(--green)}.service-hero.tone-attention:before{background:var(--orange)}
.service-symbol,.service-card-icon{display:grid;place-items:center;color:var(--blue);background:var(--blue-soft)}
.service-symbol{width:70px;height:70px;border-radius:19px}.service-copy{min-width:0}.service-copy .eyebrow{color:var(--blue);font-size:14px;font-weight:800;letter-spacing:1px}.service-copy h2{margin-top:8px;font-size:28px}.service-copy p{margin-top:9px;color:var(--muted);font-size:16px;line-height:1.55}
.service-cost{min-height:92px;padding-left:24px;border-left:1px solid var(--line);display:flex;flex-direction:column;justify-content:center}.service-cost small,.service-cost span{color:var(--muted);font-size:14px}.service-cost b{margin:8px 0;color:var(--navy);font-size:23px}.service-hero>.btn{min-width:165px}
.service-cards{display:grid;grid-template-columns:repeat(3,1fr);gap:14px}.service-card{padding:20px;display:grid;grid-template-columns:52px minmax(0,1fr);grid-template-rows:1fr auto;gap:8px 14px}.service-card-icon{width:50px;height:50px;border-radius:13px}.service-card>div:nth-child(2){min-width:0}.service-card small{color:var(--muted);font-size:14px}.service-card h3{margin-top:6px;font-size:18px}.service-card p{margin-top:7px;color:var(--muted);font-size:14px;line-height:1.45}.service-card>.btn{grid-column:1/3;min-height:38px}
.simple-explainer{padding:24px 28px;display:flex;align-items:center;justify-content:space-between;gap:30px}.simple-explainer>div{max-width:850px}.simple-explainer h2{font-size:21px}.simple-explainer p{margin-top:8px;color:var(--muted);font-size:15px;line-height:1.6}.simple-explainer>.btn{min-width:150px}
:global(:root[data-theme="dark"]) .service-cost b{color:var(--text)}
@media(max-width:1380px){.advanced-label{display:none}.service-hero{grid-template-columns:62px minmax(280px,1fr) 210px auto;padding:20px;gap:15px}.service-symbol{width:58px;height:58px}.service-copy h2{font-size:24px}.service-copy p{font-size:14px}.service-cost{padding-left:16px}.service-cost b{font-size:19px}.service-cards{gap:10px}.service-card{padding:15px}.service-page{grid-template-rows:170px 180px minmax(120px,1fr)}}
</style>
