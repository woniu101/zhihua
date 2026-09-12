<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { CalendarClock, Calculator, Database, ExternalLink, FolderOpen, HardDrive, KeyRound, Laptop, Monitor, Moon, Palette, Power, RefreshCw, Server, ShieldCheck, Sun, Wallet, Wrench, X } from "lucide-vue-next";
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
  type CompShareInstance,
  type ComputeReleaseEligibility,
  type ComputeKeepAlivePolicy,
  type ComputePolicySnapshot,
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
const llmOpen = ref(false);
const busy = ref(false);
const connectionError = ref("");
const computeForm = reactive({ publicKey: "", privateKey: "" });
const computeConfiguration = ref<CompShareConfiguration>();
const computeBalance = ref<CompShareBalance>();
const computeInstance = ref<CompShareInstance>();
const computeInstances = ref<CompShareInstance[]>([]);
const managedInstances = ref<ManagedComputeInstance[]>([]);
const releaseEligibility = ref<Record<string, ComputeReleaseEligibility>>({});
const computePolicy = ref<ComputePolicySnapshot>({ policy: "economy", idleShutdownMinutes: 3, hardLimitMinutes: 60 });
const computeNotice = ref("");
const computeNoticeTone = ref<"success" | "error" | "neutral">("neutral");
const llmConfiguration = ref<LlmConfiguration>();
const llmForm = reactive({ providerId: "deepseek" as LlmProviderId, baseUrl: "https://api.deepseek.com", model: "deepseek-chat", apiKey: "" });
const llmNotice = ref("");
const llmNoticeTone = ref<"success" | "error" | "neutral">("neutral");
const activeSection = ref<"compute" | "appearance">("compute");
const storageInfo = ref<StorageInfo>();
const storageNotice = ref("");
const computePolicyOptions: Array<{ value: ComputeKeepAlivePolicy; label: string; description: string; limit: string }> = [
  { value: "economy", label: "省费用", description: "任务落盘后空闲 3 分钟关 GPU；关闭客户端且队列为空时立即关 GPU。", limit: "上限 1 小时" },
  { value: "availability", label: "任务优先", description: "空闲 15 分钟再关 GPU，适合连续制作多个镜头，减少频繁抢卡。", limit: "上限 3 小时" },
  { value: "continuous", label: "持续 GPU", description: "不因空闲或关闭客户端而关 GPU，适合库存紧张或连续工作。", limit: "保险上限 12 小时" },
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

async function refreshCompute() {
  try {
    computePolicy.value = await compShareRepository.computePolicy();
    computeConfiguration.value = await compShareRepository.configuration();
    if (!computeConfiguration.value.credentialsStored) return;
    const [balance, instances, managed] = await Promise.all([
      compShareRepository.balance(),
      compShareRepository.listInstances(),
      compShareRepository.reconcileInstances(),
    ]);
    computeBalance.value = balance;
    computeInstances.value = instances;
    managedInstances.value = managed;
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

function ownershipLabel(ownership: ManagedComputeInstance["ownership"]) {
  return ownership === "zhihua_managed" ? "知画创建" : "用户创建";
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

async function changeComputeMode(mode: "gpu" | "noGpu" | "stop") {
  busy.value = true;
  try {
    const result = mode === "stop"
      ? await compShareRepository.stop()
      : await compShareRepository.start(mode);
    computeInstance.value = result.instance;
    setComputeNotice(
      mode === "gpu" ? "已请求启动 GPU。" : mode === "noGpu" ? "已请求无卡启动。" : "已请求关机。",
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

onMounted(() => Promise.allSettled([refreshConnection(), refreshCompute(), refreshLlm(), refreshStorage()]));
</script>

<template>
  <section class="page settings-page">
    <header class="settings-head"><div class="page-title-line"><h1>设置与算力</h1><span class="status-line"><span class="dot" :class="{ gray: !connected }"></span><strong>{{ serviceStatus }}</strong><span>|</span><span>生成任务提交后再启动 GPU</span></span></div></header>
    <nav class="settings-tabs"><button :class="{ active: activeSection === 'compute' }" type="button" @click="activeSection='compute'"><Server :size="22"/>算力与连接</button><button :class="{ active: activeSection === 'appearance' }" type="button" @click="activeSection='appearance'"><Monitor :size="22"/>存储与外观</button><span class="settings-tabs-spacer"></span><button type="button" @click="llmOpen=true"><KeyRound :size="20"/>大模型设置</button></nav>

    <div v-if="activeSection === 'compute'" class="settings-grid">
      <section class="panel account"><div class="panel-head"><h2>账户概览</h2><button class="btn link" type="button" :disabled="busy" @click="refreshCompute"><RefreshCw :size="17"/>实时刷新</button></div><div class="money-row"><article><Database :size="34"/><div><span>可用余额</span><b>{{ computeBalance?.amountAvailable ? `¥ ${computeBalance.amountAvailable}` : '--' }}</b></div></article><article><span>当前实例模式</span><b>{{ computeModeLabel }}</b><small>{{ computeConfigured ? '来自优云智算实时接口' : '请先配置算力账户' }}</small></article><article class="cost"><Calculator :size="32"/><div><span>当前实例费率</span><b>{{ computeInstance?.instancePrice == null ? '平台未返回' : `¥ ${computeInstance.instancePrice}/小时` }}</b><small>费用以优云智算实际账单为准</small></div></article></div><div class="account-actions"><button class="btn primary" type="button" @click="computeOpen=true"><Wallet :size="19"/>{{ computeConfigured ? '管理算力账户' : '配置算力账户' }}</button><button class="btn" type="button" :disabled="!computeConfigured" @click="refreshCompute"><FileTextIcon/>刷新余额与实例</button></div></section>

      <section class="panel instance"><div class="panel-head"><h2>绑定实例</h2><span :class="computeInstance?.state === 'running' ? 'success-text' : 'waiting-text'"><span class="dot" :class="{ gray: computeInstance?.state !== 'running' }"></span>{{ computeModeLabel }}</span></div><div class="instance-main"><div class="server-art">▤</div><div><h2>⌖ {{ computeInstance?.name ?? '尚未绑定实例' }}</h2><p>{{ computeInstance?.gpuType ? `RTX ${computeInstance.gpuType}` : '等待实例信息' }}</p><span class="muted"><span class="dot gray"></span>{{ computeInstance ? `${computeInstance.cpu ?? '--'} 核 · ${computeInstance.memoryMb ? Math.round(computeInstance.memoryMb / 1024) : '--'} GB` : '配置账户后选择实例' }}</span></div><span class="muted"><span class="dot gray"></span>{{ probe?.comfyuiReady ? '可生成' : '生成服务待启动' }}</span></div><div class="instance-metrics"><div><span>当前模式</span><b>{{ computeModeLabel }}</b></div><div><span>GPU 规格</span><b>{{ computeInstance?.gpuType ? `RTX ${computeInstance.gpuType}` : '--' }}</b></div><div><span>区域/可用区</span><b>{{ computeInstance?.zone ?? '--' }}</b></div></div><div class="instance-actions"><button class="btn primary" type="button" :disabled="busy || computeInstance?.state !== 'stopped'" @click="changeComputeMode('gpu')"><Power :size="18"/>启动 GPU</button><button class="btn" type="button" :disabled="busy || !computeInstance || (computeInstance.state !== 'stopped' && computeInstance.state !== 'running')" @click="computeInstance?.state === 'running' ? changeComputeMode('stop') : changeComputeMode('noGpu')"><Wrench :size="18"/>{{ computeInstance?.state === 'running' ? '关机' : '无卡启动' }}</button><button class="btn" type="button" @click="computeOpen=true"><ExternalLink :size="17"/>选择实例</button></div></section>

      <section class="panel shutdown"><div class="panel-title"><h2>GPU 保持策略</h2><span class="policy-badge" :class="{ neutral: computeInstance?.runningMode !== 'gpu' || !computeInstance?.stopSchedulerTime }">{{ policyGuardLabel }}</span></div><div class="policy-options"><button v-for="option in computePolicyOptions" :key="option.value" type="button" :class="{ selected: computePolicy.policy === option.value }" :disabled="busy" @click="changeComputePolicy(option.value)"><span><b>{{ option.label }}</b><small>{{ option.limit }}</small></span><p>{{ option.description }}</p><i>{{ computePolicy.policy === option.value ? '✓' : '' }}</i></button></div><div class="policy-guard"><CalendarClock :size="21"/><span><b>平台硬保护</b><small>{{ stopSchedulerLabel }}</small></span><button type="button" :disabled="busy || computeInstance?.state !== 'running' || computeInstance?.runningMode !== 'gpu'" @click="toggleStopDeadline">刷新保障</button></div></section>

      <section class="panel environment"><div class="panel-title"><h2>环境检查</h2><button class="btn link" type="button" :disabled="busy || (!connectionInfo?.configured && !tunnelStatus.configured)" @click="refreshConnection"><RefreshCw :size="15"/>连接并检查</button></div><div class="check-list"><p v-for="(item,index) in checks" :key="item.name"><span class="service-icon">{{ ['知','⌘','◇','▧','≋','⊞'][index] }}</span>{{ item.name }}<span :class="item.tone === 'success' ? 'success-text' : 'waiting-text'"><span class="dot" :class="{ gray: item.tone !== 'success' }"></span>{{ item.state }}</span></p></div><div class="disk"><HardDrive :size="24"/><b>远端磁盘</b><span>等待优云智算实例接口</span><div class="progress"><i style="width:0"></i></div><strong>未知</strong></div></section>

      <section class="panel retention"><div class="panel-title"><h2>实例保留期　ⓘ</h2><span class="policy-badge neutral">以平台时间为准</span></div><div class="setting-row"><CalendarClock :size="24"/><div><b>平台回收时间</b><span>关机实例可能进入平台回收倒计时；运行或启动实例时会重新查询。</span></div><strong>{{ releaseTimeLabel }}</strong></div><div class="warning-box"><b>实例不会被静默释放</b><span>主实例和用户创建的实例永久保留；仅知画创建且已关机、无任务的弹性实例可在确认后释放。</span></div></section>

      <section class="panel connection"><div class="panel-head"><h2>连接信息</h2><span :class="connected ? 'success-text' : 'waiting-text'"><span class="dot" :class="{ gray: !connected }"></span>{{ connectionLabel }}</span></div><div class="connection-cards"><article role="button" tabindex="0" @click="llmOpen=true" @keydown.enter="llmOpen=true"><KeyRound :size="31"/><div><b>大模型内容规划</b><span>{{ llmConfiguration?.credentialStored ? `${llmConfiguration.providerLabel} · ${llmConfiguration.model}` : '等待配置 API Key' }}</span></div><strong>›</strong></article><article role="button" tabindex="0" @click="refreshConnection" @keydown.enter="refreshConnection"><KeyRound :size="31"/><div><b>自动安全连接</b><span>{{ tunnelStatus.configured ? 'SSH 私钥已保存到 Windows 凭据库' : '等待实例连接配置' }}</span></div><strong>›</strong></article><article role="button" tabindex="0" @click="refreshConnection" @keydown.enter="refreshConnection"><Server :size="31"/><div><b>本机服务入口</b><span>{{ tunnelStatus.localUrl ?? connectionInfo?.baseUrl ?? '启动时自动分配' }}</span></div><strong>›</strong></article><article role="button" tabindex="0" @click="refreshConnection" @keydown.enter="refreshConnection"><ShieldCheck :size="31"/><div><b>版本握手　<span :class="connected ? 'success-text' : 'waiting-text'">● {{ connectionLabel }}</span></b><span>{{ connected ? `API ${probe?.apiVersion} · 服务 ${probe?.serviceVersion}` : connectionError || '点击建立连接并检查兼容性' }}</span></div><strong>›</strong></article></div></section>
    </div>

    <div v-else class="appearance-grid">
      <section class="panel theme-panel">
        <div class="panel-head"><div><h2>界面主题</h2><p>切换后立即应用到全部页面，并保存在本机。</p></div><span class="theme-current"><Palette :size="18"/>当前显示：{{ effectiveTheme === 'dark' ? '深色' : '浅色' }}</span></div>
        <div class="theme-options">
          <button v-for="option in themeOptions" :key="option.value" type="button" :class="{ selected: themePreference === option.value }" @click="setThemePreference(option.value)">
            <span class="theme-icon"><component :is="option.icon" :size="28"/></span><span><b>{{ option.label }}</b><small>{{ option.description }}</small></span><i>{{ themePreference === option.value ? '✓' : '' }}</i>
          </button>
        </div>
        <div class="theme-preview" :class="`preview-${effectiveTheme}`"><div class="preview-nav"><span></span><i></i><i></i><i></i></div><div class="preview-main"><header></header><section><article></article><article></article><article></article></section></div></div>
      </section>

      <section class="panel local-storage-panel">
        <div class="panel-head"><div><h2>本地存储</h2><p>项目、素材、候选视频和导出临时文件都保存在本机。</p></div><HardDrive :size="24"/></div>
        <div class="storage-path-card"><span><FolderOpen :size="25"/></span><div><b>项目根目录</b><code>{{ storageInfo?.projectsRoot ?? '正在读取…' }}</code></div></div>
        <div class="storage-path-card"><span><Database :size="25"/></span><div><b>项目索引数据库</b><code>{{ storageInfo?.databasePath ?? '正在读取…' }}</code></div></div>
        <div class="storage-meta"><span>数据库结构版本</span><b>v{{ storageInfo?.schemaVersion ?? '--' }}</b></div>
        <p v-if="storageNotice" class="storage-notice">{{ storageNotice }}</p>
        <button class="btn primary open-storage" type="button" :disabled="!storageInfo" @click="openProjectsRoot"><FolderOpen :size="19"/>打开项目目录</button>
      </section>

      <section class="panel appearance-note"><Palette :size="26"/><div><h2>外观只影响显示</h2><p>切换主题不会中断生成任务，也不会改变图片、视频和导出成片的颜色。</p></div></section>
    </div>

    <div v-if="computeOpen" class="connection-backdrop" role="presentation" @click.self="computeOpen=false">
      <section class="connection-dialog compute-dialog" role="dialog" aria-modal="true" aria-labelledby="compute-title">
        <header><div><h2 id="compute-title">优云智算实例中心</h2><p>平台实例会自动对账；用户实例只控制运行状态，知画弹性实例才允许安全释放。</p></div><button type="button" aria-label="关闭" @click="computeOpen=false"><X :size="20"/></button></header>
        <div class="connection-form">
          <template v-if="!computeConfigured">
            <label><span>API 公钥</span><input v-model="computeForm.publicKey" autocomplete="off" placeholder="输入 PublicKey"/></label>
            <label><span>API 私钥</span><input v-model="computeForm.privateKey" type="password" autocomplete="new-password" placeholder="输入 PrivateKey"/></label>
          </template>
          <template v-else>
            <p class="compute-summary">账户已连接　·　可用余额 <b>{{ computeBalance?.amountAvailable ? `¥ ${computeBalance.amountAvailable}` : '--' }}</b></p>
            <div class="instance-center-head"><span>共 {{ managedInstances.length }} 个实例</span><small>单击“设为主实例”后用于日常生成</small></div>
            <div class="instance-picker managed-picker">
              <article v-for="item in managedInstances" :key="item.instanceId" :class="{ selected: item.instanceId === computeConfiguration?.boundInstanceId, uncertain: item.lifecycleState === 'unknown' }">
                <div class="instance-row-main">
                  <span class="instance-role" :class="item.ownership === 'zhihua_managed' ? 'managed' : ''">{{ roleLabel(item.role) }}</span>
                  <div><b>{{ item.name ?? item.instanceId }}</b><small>{{ item.zone }} · {{ item.gpuType ? `RTX ${item.gpuType}` : 'GPU 规格待查询' }} · {{ ownershipLabel(item.ownership) }}</small></div>
                  <strong>{{ managedModeLabel(item) }}</strong>
                </div>
                <div class="instance-row-foot"><span>{{ formatReleaseTime(item.releaseTime) }}</span><span v-if="item.missingSince" class="danger-text">平台暂未返回，等待对账</span><div><button v-if="item.instanceId !== computeConfiguration?.boundInstanceId" class="mini-btn" type="button" :disabled="busy || item.lifecycleState === 'unknown'" @click="bindManagedInstance(item)">设为主实例</button><span v-else class="selected-label">当前主实例</span><button v-if="releaseEligibility[item.instanceId]?.allowed" class="mini-btn danger" type="button" :disabled="busy" @click="releaseManagedInstance(item)">释放</button></div></div>
              </article>
              <p v-if="!managedInstances.length" class="empty-instances">平台没有返回可用实例。创建弹性实例前会先检查地域库存和实时报价，并再次让你确认。</p>
            </div>
          </template>
          <p v-if="computeNotice" class="connection-notice" :class="computeNoticeTone">{{ computeNotice }}</p>
        </div>
        <footer><span></span><span></span><button class="btn" type="button" :disabled="busy" @click="refreshCompute">刷新</button><button v-if="!computeConfigured" class="btn primary" type="button" :disabled="busy" @click="saveComputeCredentials">保存并验证</button><button v-else class="btn primary" type="button" @click="computeOpen=false">完成</button></footer>
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
.settings-page{display:grid;grid-template-rows:78px 61px minmax(0,1fr);gap:0}.settings-head{display:flex;align-items:center;padding-left:16px}.settings-tabs{display:flex;gap:30px;border-bottom:1px solid var(--line);padding:0 20px}.settings-tabs button{height:61px;padding:0 12px;border:0;background:transparent;display:flex;align-items:center;gap:12px;font-size:17px;font-weight:700;color:#29446f;position:relative}.settings-tabs button.active{color:var(--blue)}.settings-tabs button.active:after{content:"";position:absolute;left:0;right:0;bottom:0;height:4px;border-radius:4px;background:var(--blue)}.settings-grid{min-height:0;padding-top:14px;display:grid;grid-template-columns:1.05fr 1fr 1fr;grid-template-rows:294px minmax(288px,1fr) 160px;gap:13px}.account{grid-column:1/3}.instance{grid-column:3/4}.connection{grid-column:1/4}.panel-head>span,.panel-title>span{display:flex;align-items:center;gap:8px;font-size:13px}.money-row{height:126px;margin:10px 18px 12px;display:grid;grid-template-columns:1fr .85fr 1.05fr;border-radius:9px;overflow:hidden;background:linear-gradient(110deg,#eaf3ff,#f7fbff)}.money-row article{display:flex;align-items:center;gap:16px;padding:16px 20px;border-right:1px solid #d7e2f0}.money-row svg{color:var(--blue)}.money-row article>div,.money-row article{color:#1b3159}.money-row span{display:block;font-size:14px}.money-row b{display:block;margin-top:8px;font-size:31px;color:#07183d}.money-row b small{font-size:13px}.money-row .cost{background:linear-gradient(120deg,#fff8ef,#fffaf5);color:#263d64}.money-row .cost svg,.money-row .cost b{color:#f26b0f}.money-row .cost small{display:block;margin-top:5px;color:#677b9d;font-size:11px}.account-actions{display:grid;grid-template-columns:1fr 1fr;gap:14px;padding:0 18px}.account-actions .btn{height:50px;font-size:17px}.instance-main{height:105px;display:grid;grid-template-columns:104px 1fr auto;align-items:center;padding:8px 18px;gap:14px}.server-art{width:100px;height:85px;display:grid;place-items:center;background:linear-gradient(145deg,#d7e7f7,#f8fbff);border-radius:8px;font-size:62px;color:#274b73}.instance-main h2{font-size:20px}.instance-main p{font-size:17px;margin:5px 0}.instance-main span{display:flex;align-items:center;gap:7px;font-size:14px}.instance-main>span{align-self:center}.instance-metrics{height:66px;display:grid;grid-template-columns:.8fr 1.25fr 1fr;border-top:1px solid #e1e8f1;padding:8px 16px}.instance-metrics>div{display:flex;flex-direction:column;border-right:1px solid #dfe6ef;padding-left:8px}.instance-metrics>div:last-child{border:0}.instance-metrics span{font-size:12px;color:#617596}.instance-metrics b{font-size:16px;margin-top:5px}.instance-actions{display:grid;grid-template-columns:.8fr 1fr 1.55fr;gap:9px;padding:6px 18px}.instance-actions .btn{min-height:40px;padding:0 8px}.panel-title{height:55px;padding:0 21px;display:flex;align-items:center;justify-content:space-between}.switch{width:49px;height:27px;border-radius:20px;background:#cbd6e6;position:relative}.switch:after{content:"";position:absolute;left:3px;top:3px;width:21px;height:21px;border-radius:50%;background:white;box-shadow:0 1px 4px #9aabc2}.switch.on{background:var(--blue)}.switch.on:after{left:25px}.setting-row{min-height:74px;margin:0 22px;border-bottom:1px solid #e0e7f1;display:flex;align-items:center;gap:15px}.setting-row svg{color:#16355f}.setting-row>div{display:flex;flex-direction:column;gap:6px;flex:1}.setting-row b{font-size:15px}.setting-row span{font-size:12px;color:#687c9e}.setting-row button{height:43px;min-width:150px;border:1px solid #cddbee;border-radius:7px;background:#fff}.check-list{padding:0 18px}.check-list p{height:33px;display:flex;align-items:center;gap:12px;border-bottom:1px solid #e7edf4;font-size:14px}.check-list p>span:last-child{margin-left:auto;display:flex;align-items:center;gap:7px}.service-icon{width:23px;height:23px;display:grid;place-items:center;color:#19407c;font-weight:800}.disk{height:57px;display:grid;grid-template-columns:30px 92px 1fr 120px 36px;align-items:center;gap:8px;padding:0 18px}.disk>span{font-size:12px;color:#5f7394}.warning-box{margin:12px 17px;padding:15px;border-radius:8px;background:#fff3e4;color:#ed6a0d;display:flex;flex-direction:column;gap:6px}.warning-box span{font-size:12px;color:#745d4e}.connection-cards{height:102px;display:grid;grid-template-columns:repeat(4,1fr);gap:10px;padding:12px 18px}.connection-cards article{border:1px solid #dbe4f0;border-radius:8px;display:grid;grid-template-columns:42px 1fr 15px;align-items:center;padding:0 13px;min-width:0}.connection-cards svg{color:var(--blue)}.connection-cards article>div{display:flex;flex-direction:column;gap:6px;min-width:0}.connection-cards span{font-size:12px;color:#687c9e;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.connection-cards .success-text{font-size:12px}.retention .setting-row{min-height:70px}@media(max-width:1380px){.settings-grid{grid-template-rows:270px minmax(260px,1fr) 145px}.settings-tabs{gap:14px}.money-row{height:112px}.money-row b{font-size:25px}.instance-main{height:92px}.server-art{width:80px;height:70px}.instance-actions{padding-left:10px;padding-right:10px}.connection-cards{height:88px}.connection-cards article{grid-template-columns:34px 1fr 12px;padding:0 9px}}
.waiting-text{color:#6e809d}
.appearance-grid{min-height:0;padding-top:14px;display:grid;grid-template-columns:minmax(640px,1.45fr) minmax(440px,1fr);grid-template-rows:minmax(0,1fr) 92px;gap:13px}.theme-panel,.local-storage-panel{overflow:hidden}.theme-panel>.panel-head,.local-storage-panel>.panel-head{min-height:74px}.theme-panel>.panel-head>div p,.local-storage-panel>.panel-head>div p{margin-top:5px;color:var(--muted);font-size:12px}.theme-current{display:flex;align-items:center;gap:7px;color:var(--blue);font-size:13px;font-weight:700}.theme-options{padding:18px;display:grid;grid-template-columns:repeat(3,1fr);gap:12px}.theme-options>button{min-height:96px;padding:14px;border:1px solid var(--line);border-radius:10px;background:var(--surface);display:grid;grid-template-columns:47px 1fr 20px;align-items:center;gap:10px;text-align:left}.theme-options>button:hover{border-color:#94baf1}.theme-options>button.selected{border-color:var(--blue);box-shadow:0 0 0 1px var(--blue) inset;background:var(--blue-soft)}.theme-icon{width:44px;height:44px;border-radius:10px;display:grid;place-items:center;color:var(--blue);background:var(--blue-soft)}.theme-options b,.theme-options small{display:block}.theme-options b{font-size:17px}.theme-options small{margin-top:6px;color:var(--muted);font-size:12px;line-height:1.4}.theme-options i{font-style:normal;color:var(--blue);font-weight:800}.theme-preview{height:calc(100% - 206px);min-height:210px;margin:0 18px 18px;border:1px solid var(--line);border-radius:12px;overflow:hidden;display:grid;grid-template-columns:112px 1fr;box-shadow:0 12px 30px rgba(25,62,111,.12)}.theme-preview.preview-light{background:#f4f8fe}.theme-preview.preview-dark{background:#101a2b;border-color:#30415a}.preview-nav{padding:18px 14px;display:flex;flex-direction:column;gap:12px;background:rgba(255,255,255,.9);border-right:1px solid #dce6f3}.preview-dark .preview-nav{background:#111d2f;border-color:#2b3c55}.preview-nav span{height:28px;border-radius:7px;background:var(--blue)}.preview-nav i{height:15px;border-radius:5px;background:#dfe8f5}.preview-dark .preview-nav i{background:#293951}.preview-main{padding:18px}.preview-main header{height:32px;width:45%;margin-bottom:15px;border-radius:6px;background:#d9e5f5}.preview-dark .preview-main header{background:#293b56}.preview-main section{display:grid;grid-template-columns:repeat(3,1fr);gap:12px}.preview-main article{height:115px;border:1px solid #d8e3f1;border-radius:9px;background:#fff}.preview-dark .preview-main article{border-color:#30415a;background:#18263a}.local-storage-panel{padding-bottom:18px}.local-storage-panel>.panel-head svg{color:var(--blue)}.storage-path-card{min-height:91px;margin:14px 18px 0;padding:13px;border:1px solid var(--line);border-radius:9px;display:grid;grid-template-columns:46px minmax(0,1fr);align-items:center;gap:12px;background:var(--surface-soft)}.storage-path-card>span{width:43px;height:43px;border-radius:9px;display:grid;place-items:center;color:var(--blue);background:var(--blue-soft)}.storage-path-card b,.storage-path-card code{display:block}.storage-path-card code{margin-top:7px;color:var(--muted);font:12px/1.4 Consolas,monospace;user-select:text;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.storage-meta{height:50px;margin:14px 18px 0;padding:0 14px;border:1px solid var(--line);border-radius:8px;display:flex;align-items:center;justify-content:space-between}.storage-meta span{color:var(--muted)}.storage-notice{margin:10px 18px;color:#c5483e;font-size:12px}.open-storage{width:calc(100% - 36px);height:48px;margin:14px 18px 0}.appearance-note{grid-column:1/3;display:flex;align-items:center;gap:15px;padding:0 22px}.appearance-note>svg{color:var(--blue)}.appearance-note p{margin-top:5px;color:var(--muted);font-size:13px}@media(max-width:1380px){.appearance-grid{grid-template-columns:1.35fr 1fr}.theme-options{padding:13px}.theme-options>button{padding:10px;grid-template-columns:40px 1fr 16px}.theme-icon{width:38px;height:38px}.theme-preview{height:calc(100% - 190px);margin:0 13px 13px}.storage-path-card{margin-left:13px;margin-right:13px}.open-storage{width:calc(100% - 26px);margin-left:13px;margin-right:13px}}
.settings-tabs button:disabled{opacity:.48;cursor:not-allowed}.connection-cards article[role="button"]{cursor:pointer;transition:.15s ease}.connection-cards article[role="button"]:hover,.connection-cards article[role="button"]:focus-visible{border-color:#8eb8f7;background:#f7faff;outline:0}.connection-backdrop{position:fixed;z-index:80;inset:30px 0 0 0;background:rgba(6,20,46,.32);display:grid;place-items:center;padding:24px}.connection-dialog{width:min(620px,calc(100vw - 80px));border:1px solid #cedbed;border-radius:13px;background:#fff;box-shadow:0 24px 70px rgba(16,45,88,.24);overflow:hidden}.connection-dialog>header{min-height:88px;padding:19px 22px;display:flex;align-items:flex-start;justify-content:space-between;border-bottom:1px solid var(--line);background:linear-gradient(135deg,#f7fbff,#fff)}.connection-dialog>header h2{font-size:22px}.connection-dialog>header p{margin-top:7px;color:#64799b;font-size:13px}.connection-dialog>header button{width:34px;height:34px;border:0;border-radius:7px;background:transparent;display:grid;place-items:center}.connection-dialog>header button:hover{background:#edf3fb}.connection-form{padding:20px 22px;display:flex;flex-direction:column;gap:16px}.connection-form label{display:grid;grid-template-columns:132px minmax(0,1fr);align-items:center;gap:8px 14px}.connection-form label>span{font-weight:700;color:#1b355f}.connection-form input,.connection-form select{height:43px;border:1px solid #cbd9ec;border-radius:8px;padding:0 12px;background:#fff;user-select:text}.connection-form input:focus,.connection-form select:focus{outline:2px solid #cfe2ff;border-color:var(--blue)}.connection-form small{grid-column:2;color:#6d809f;font-size:12px}.connection-notice{margin:2px 0 0 146px;padding:10px 12px;border-radius:7px;background:#f1f5fa;color:#52698e;font-size:13px}.connection-notice.success{background:#e8f8f1;color:#087d51}.connection-notice.error{background:#fff0ee;color:#b33b35}.connection-dialog>footer{min-height:72px;padding:13px 22px;border-top:1px solid var(--line);display:grid;grid-template-columns:auto 1fr auto auto;align-items:center;gap:10px;background:#fbfdff}.connection-dialog button:disabled,.environment button:disabled{opacity:.55;cursor:not-allowed}.compute-summary{margin:0;padding:13px 15px;border-radius:8px;background:#eef6ff;color:#29486f}.compute-summary b{color:var(--blue);font-size:20px}.instance-picker{display:flex;flex-direction:column;gap:9px;max-height:280px;overflow:auto}.instance-picker>button{min-height:68px;padding:10px 14px;border:1px solid #d7e2ef;border-radius:8px;background:#fff;display:flex;align-items:center;justify-content:space-between;text-align:left;color:#17345f}.instance-picker>button.selected{border-color:var(--blue);background:#f1f7ff;box-shadow:0 0 0 1px var(--blue) inset}.instance-picker>button span{display:flex;flex-direction:column;gap:6px}.instance-picker>button small{color:#7183a0}.instance-picker>button strong{color:#315d9b;font-size:13px}
.settings-tabs-spacer{flex:1}.policy-badge{padding:5px 9px;border-radius:14px;background:#e8f8f1;color:#087d51;font-size:12px;font-weight:700}.policy-badge.neutral{background:var(--surface-soft);color:var(--muted)}
.policy-options{padding:0 14px;display:grid;gap:7px}.policy-options>button{min-height:55px;padding:7px 30px 7px 10px;border:1px solid #dbe4f0;border-radius:8px;background:#f9fbfe;text-align:left;position:relative;color:var(--text)}.policy-options>button.selected{border-color:var(--blue);background:#edf5ff;box-shadow:0 0 0 1px rgba(15,105,255,.08)}.policy-options>button>span{display:flex;align-items:center;justify-content:space-between;gap:8px}.policy-options b{font-size:13px}.policy-options small{color:var(--blue);font-size:10px;font-weight:700}.policy-options p{margin-top:3px;color:#657a9a;font-size:10px;line-height:1.3}.policy-options i{position:absolute;right:10px;top:19px;width:18px;height:18px;border-radius:50%;display:grid;place-items:center;background:var(--blue);color:white;font-style:normal}.policy-guard{height:48px;margin:7px 14px 0;padding-top:7px;border-top:1px solid #e1e8f1;display:grid;grid-template-columns:27px minmax(0,1fr) 72px;align-items:center;gap:5px}.policy-guard>svg{color:#16355f}.policy-guard>span{min-width:0;display:flex;flex-direction:column}.policy-guard b{font-size:11px}.policy-guard small{color:#657a9a;font-size:9px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.policy-guard button{height:30px;border:1px solid #cfdbea;border-radius:6px;background:#fff;color:#31537f;font-size:11px}
.compute-dialog{width:min(790px,calc(100vw - 80px))}.compute-dialog .connection-form{max-height:min(610px,calc(100vh - 250px));overflow:auto}.instance-center-head{display:flex;align-items:center;justify-content:space-between;color:#23446f;font-weight:700}.instance-center-head small{color:#7183a0;font-weight:400}.managed-picker{max-height:390px}.managed-picker>article{padding:13px 14px;border:1px solid #d7e2ef;border-radius:9px;background:#fff}.managed-picker>article.selected{border-color:var(--blue);background:#f4f8ff;box-shadow:0 0 0 1px var(--blue) inset}.managed-picker>article.uncertain{border-style:dashed;background:#fafbfc}.instance-row-main{display:grid;grid-template-columns:72px minmax(0,1fr) auto;align-items:center;gap:12px}.instance-row-main>div{min-width:0}.instance-row-main b,.instance-row-main small{display:block}.instance-row-main b{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#17345f}.instance-row-main small{margin-top:5px;color:#7183a0;font-size:12px}.instance-row-main strong{color:#315d9b;font-size:13px}.instance-role{padding:5px 7px;border-radius:13px;background:#edf1f6;color:#5e708d;text-align:center;font-size:11px;font-weight:700}.instance-role.managed{background:#e9f8f1;color:#087d51}.instance-row-foot{margin-top:10px;padding-top:9px;border-top:1px solid #e6edf5;display:flex;align-items:center;gap:12px;color:#7183a0;font-size:11px}.instance-row-foot>div{margin-left:auto;display:flex;align-items:center;gap:8px}.selected-label{color:var(--blue);font-weight:700}.mini-btn{height:29px;padding:0 10px;border:1px solid #b9cce5;border-radius:6px;background:#fff;color:#285184;font-size:11px}.mini-btn.danger{border-color:#edb4ad;color:#b83830}.danger-text{color:#bd463d}.empty-instances{padding:24px 18px;border:1px dashed #cbd9eb;border-radius:9px;color:#6a7f9f;line-height:1.6;text-align:center}
</style>
