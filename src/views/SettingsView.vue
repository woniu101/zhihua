<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { CalendarClock, Calculator, Database, ExternalLink, Gauge, HardDrive, KeyRound, Monitor, Power, RefreshCw, Server, ShieldCheck, Timer, Wallet, Wrench, X } from "lucide-vue-next";
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
} from "../services/compShareRepository";
import {
  sshTunnelRepository,
  type TunnelStatus,
} from "../services/sshTunnelRepository";
import {
  deepSeekRepository,
  normalizeDeepSeekError,
  type DeepSeekConfiguration,
} from "../services/deepSeekRepository";

const connectionInfo = ref<ServiceConnectionInfo>();
const probe = ref<ServiceProbe>();
const tunnelStatus = ref<TunnelStatus>({ configured: false, phase: "stopped" });
const computeOpen = ref(false);
const deepSeekOpen = ref(false);
const busy = ref(false);
const connectionError = ref("");
const computeForm = reactive({ publicKey: "", privateKey: "" });
const computeConfiguration = ref<CompShareConfiguration>();
const computeBalance = ref<CompShareBalance>();
const computeInstance = ref<CompShareInstance>();
const computeInstances = ref<CompShareInstance[]>([]);
const computeNotice = ref("");
const computeNoticeTone = ref<"success" | "error" | "neutral">("neutral");
const deepSeekConfiguration = ref<DeepSeekConfiguration>();
const deepSeekForm = reactive({ baseUrl: "https://api.deepseek.com", model: "deepseek-chat", apiKey: "" });
const deepSeekNotice = ref("");
const deepSeekNoticeTone = ref<"success" | "error" | "neutral">("neutral");

const computeConfigured = computed(() => Boolean(computeConfiguration.value?.credentialsStored));
const deepSeekConfigured = computed(() => Boolean(deepSeekConfiguration.value?.credentialStored));
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
    computeConfiguration.value = await compShareRepository.configuration();
    if (!computeConfiguration.value.credentialsStored) return;
    const [balance, instances] = await Promise.all([
      compShareRepository.balance(),
      compShareRepository.listInstances(),
    ]);
    computeBalance.value = balance;
    computeInstances.value = instances;
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
    setComputeNotice("账户验证成功，密钥已保存到 Windows 凭据管理器。", "success");
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
    if (computeInstance.value.stopSchedulerTime) {
      const result = await compShareRepository.clearStopDeadline();
      computeInstance.value = result.instance;
      setComputeNotice("已取消平台定时关机。", "success");
    } else {
      const result = await compShareRepository.setStopDeadline(
        Math.floor(Date.now() / 1000) + 60 * 60,
        computeConfiguration.value?.projectId,
      );
      computeInstance.value = result.instance;
      setComputeNotice("已设置 60 分钟平台定时关机保障。", "success");
    }
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

async function refreshDeepSeek() {
  try {
    deepSeekConfiguration.value = await deepSeekRepository.configuration();
    deepSeekForm.baseUrl = deepSeekConfiguration.value.baseUrl;
    deepSeekForm.model = deepSeekConfiguration.value.model;
  } catch (error) {
    deepSeekNotice.value = normalizeDeepSeekError(error).message;
    deepSeekNoticeTone.value = "error";
  }
}

async function saveOrTestDeepSeek() {
  if (!deepSeekConfigured.value && !deepSeekForm.apiKey.trim()) {
    deepSeekNotice.value = "请填写 DeepSeek API Key。";
    deepSeekNoticeTone.value = "error";
    return;
  }
  busy.value = true;
  deepSeekNotice.value = "正在验证 DeepSeek 连接……";
  deepSeekNoticeTone.value = "neutral";
  try {
    if (deepSeekForm.apiKey.trim()) {
      deepSeekConfiguration.value = await deepSeekRepository.save(
        deepSeekForm.baseUrl,
        deepSeekForm.model,
        deepSeekForm.apiKey.trim(),
      );
      deepSeekForm.apiKey = "";
    }
    const tested = await deepSeekRepository.test();
    deepSeekNotice.value = `连接成功，内容规划将使用 ${tested.model}。`;
    deepSeekNoticeTone.value = "success";
  } catch (error) {
    deepSeekNotice.value = normalizeDeepSeekError(error).message;
    deepSeekNoticeTone.value = "error";
  } finally {
    busy.value = false;
  }
}

async function clearDeepSeek() {
  await deepSeekRepository.clear();
  await refreshDeepSeek();
  deepSeekNotice.value = "已从 Windows 凭据管理器移除 DeepSeek API Key。";
  deepSeekNoticeTone.value = "neutral";
}

onMounted(() => Promise.allSettled([refreshConnection(), refreshCompute(), refreshDeepSeek()]));
</script>

<template>
  <section class="page settings-page">
    <header class="settings-head"><div class="page-title-line"><h1>设置与算力</h1><span class="status-line"><span class="dot" :class="{ gray: !connected }"></span><strong>{{ serviceStatus }}</strong><span>|</span><span>生成任务提交后再启动 GPU</span></span></div></header>
    <nav class="settings-tabs"><button type="button" @click="deepSeekOpen=true"><KeyRound :size="22"/>连接状态</button><button class="active" type="button"><Server :size="22"/>算力实例</button><button type="button" title="环境状态显示在下方"><ShieldCheck :size="22"/>环境检查</button><button type="button" disabled title="将在语音与导出阶段开放"><Gauge :size="22"/>语音与导出</button><button type="button" disabled title="将在本地设置阶段开放"><Monitor :size="22"/>存储与外观</button></nav>

    <div class="settings-grid">
      <section class="panel account"><div class="panel-head"><h2>账户概览</h2><button class="btn link" type="button" :disabled="busy" @click="refreshCompute"><RefreshCw :size="17"/>实时刷新</button></div><div class="money-row"><article><Database :size="34"/><div><span>可用余额</span><b>{{ computeBalance?.amountAvailable ? `¥ ${computeBalance.amountAvailable}` : '--' }}</b></div></article><article><span>当前实例模式</span><b>{{ computeModeLabel }}</b><small>{{ computeConfigured ? '来自优云智算实时接口' : '请先配置算力账户' }}</small></article><article class="cost"><Calculator :size="32"/><div><span>当前实例费率</span><b>{{ computeInstance?.instancePrice == null ? '平台未返回' : `¥ ${computeInstance.instancePrice}/小时` }}</b><small>费用以优云智算实际账单为准</small></div></article></div><div class="account-actions"><button class="btn primary" type="button" @click="computeOpen=true"><Wallet :size="19"/>{{ computeConfigured ? '管理算力账户' : '配置算力账户' }}</button><button class="btn" type="button" :disabled="!computeConfigured" @click="refreshCompute"><FileTextIcon/>刷新余额与实例</button></div></section>

      <section class="panel instance"><div class="panel-head"><h2>绑定实例</h2><span :class="computeInstance?.state === 'running' ? 'success-text' : 'waiting-text'"><span class="dot" :class="{ gray: computeInstance?.state !== 'running' }"></span>{{ computeModeLabel }}</span></div><div class="instance-main"><div class="server-art">▤</div><div><h2>⌖ {{ computeInstance?.name ?? '尚未绑定实例' }}</h2><p>{{ computeInstance?.gpuType ? `RTX ${computeInstance.gpuType}` : '等待实例信息' }}</p><span class="muted"><span class="dot gray"></span>{{ computeInstance ? `${computeInstance.cpu ?? '--'} 核 · ${computeInstance.memoryMb ? Math.round(computeInstance.memoryMb / 1024) : '--'} GB` : '配置账户后选择实例' }}</span></div><span class="muted"><span class="dot gray"></span>{{ probe?.comfyuiReady ? '可生成' : '生成服务待启动' }}</span></div><div class="instance-metrics"><div><span>当前模式</span><b>{{ computeModeLabel }}</b></div><div><span>GPU 规格</span><b>{{ computeInstance?.gpuType ? `RTX ${computeInstance.gpuType}` : '--' }}</b></div><div><span>区域/可用区</span><b>{{ computeInstance?.zone ?? '--' }}</b></div></div><div class="instance-actions"><button class="btn primary" type="button" :disabled="busy || computeInstance?.state !== 'stopped'" @click="changeComputeMode('gpu')"><Power :size="18"/>启动 GPU</button><button class="btn" type="button" :disabled="busy || !computeInstance || (computeInstance.state !== 'stopped' && computeInstance.state !== 'running')" @click="computeInstance?.state === 'running' ? changeComputeMode('stop') : changeComputeMode('noGpu')"><Wrench :size="18"/>{{ computeInstance?.state === 'running' ? '关机' : '无卡启动' }}</button><button class="btn" type="button" @click="computeOpen=true"><ExternalLink :size="17"/>选择实例</button></div></section>

      <section class="panel shutdown"><div class="panel-title"><h2>自动关机</h2><span class="switch on"></span></div><div class="setting-row"><Timer :size="25"/><div><b>空闲 3 分钟后关机</b><span>实例在设定的空闲时间后自动关机，节省费用。</span></div></div><div class="setting-row"><ClockIcon/><div><b>本次运行上限</b><span>达到时长后自动关机，避免超额费用。</span></div><button>60 分钟　⌄</button></div><div class="setting-row"><CalendarClock :size="24"/><div><b>平台定时关机</b><span>{{ stopSchedulerLabel }}</span></div><button type="button" :disabled="busy || computeInstance?.state !== 'running'" @click="toggleStopDeadline">{{ computeInstance?.stopSchedulerTime ? '取消' : '设置 60 分钟' }}</button></div></section>

      <section class="panel environment"><div class="panel-title"><h2>环境检查</h2><button class="btn link" type="button" :disabled="busy || (!connectionInfo?.configured && !tunnelStatus.configured)" @click="refreshConnection"><RefreshCw :size="15"/>连接并检查</button></div><div class="check-list"><p v-for="(item,index) in checks" :key="item.name"><span class="service-icon">{{ ['知','⌘','◇','▧','≋','⊞'][index] }}</span>{{ item.name }}<span :class="item.tone === 'success' ? 'success-text' : 'waiting-text'"><span class="dot" :class="{ gray: item.tone !== 'success' }"></span>{{ item.state }}</span></p></div><div class="disk"><HardDrive :size="24"/><b>远端磁盘</b><span>等待优云智算实例接口</span><div class="progress"><i style="width:0"></i></div><strong>未知</strong></div></section>

      <section class="panel retention"><div class="panel-title"><h2>实例保留期　ⓘ</h2></div><div class="setting-row"><CalendarClock :size="24"/><div><b>回收时间</b><span>由平台策略决定，可能在实例闲置后回收。</span></div><strong>未知</strong></div><div class="setting-row"><Wrench :size="24"/><div><b>软件运行时自动维护实例保留期</b><span>在软件运行时自动延长实例保留期。</span></div><span class="switch"></span></div><div class="warning-box"><b>尚未完成平台验证，当前不可开启</b><span>实例保留时长以优云智算平台规则为准。</span></div></section>

      <section class="panel connection"><div class="panel-head"><h2>连接信息</h2><span :class="connected ? 'success-text' : 'waiting-text'"><span class="dot" :class="{ gray: !connected }"></span>{{ connectionLabel }}</span></div><div class="connection-cards"><article role="button" tabindex="0" @click="deepSeekOpen=true" @keydown.enter="deepSeekOpen=true"><KeyRound :size="31"/><div><b>DeepSeek 内容规划</b><span>{{ deepSeekConfigured ? `${deepSeekConfiguration?.model} · API Key 已安全保存` : '等待配置 API Key' }}</span></div><strong>›</strong></article><article role="button" tabindex="0" @click="refreshConnection" @keydown.enter="refreshConnection"><KeyRound :size="31"/><div><b>自动安全连接</b><span>{{ tunnelStatus.configured ? 'SSH 私钥已保存到 Windows 凭据库' : '等待实例连接配置' }}</span></div><strong>›</strong></article><article role="button" tabindex="0" @click="refreshConnection" @keydown.enter="refreshConnection"><Server :size="31"/><div><b>本机服务入口</b><span>{{ tunnelStatus.localUrl ?? connectionInfo?.baseUrl ?? '启动时自动分配' }}</span></div><strong>›</strong></article><article role="button" tabindex="0" @click="refreshConnection" @keydown.enter="refreshConnection"><ShieldCheck :size="31"/><div><b>版本握手　<span :class="connected ? 'success-text' : 'waiting-text'">● {{ connectionLabel }}</span></b><span>{{ connected ? `API ${probe?.apiVersion} · 服务 ${probe?.serviceVersion}` : connectionError || '点击建立连接并检查兼容性' }}</span></div><strong>›</strong></article></div></section>
    </div>

    <div v-if="computeOpen" class="connection-backdrop" role="presentation" @click.self="computeOpen=false">
      <section class="connection-dialog" role="dialog" aria-modal="true" aria-labelledby="compute-title">
        <header><div><h2 id="compute-title">优云智算账户与实例</h2><p>API 密钥只保存到 Windows 凭据管理器；客户端仅控制已选择的实例。</p></div><button type="button" aria-label="关闭" @click="computeOpen=false"><X :size="20"/></button></header>
        <div class="connection-form">
          <template v-if="!computeConfigured">
            <label><span>API 公钥</span><input v-model="computeForm.publicKey" autocomplete="off" placeholder="输入 PublicKey"/></label>
            <label><span>API 私钥</span><input v-model="computeForm.privateKey" type="password" autocomplete="new-password" placeholder="输入 PrivateKey"/></label>
          </template>
          <template v-else>
            <p class="compute-summary">账户已连接　·　可用余额 <b>{{ computeBalance?.amountAvailable ? `¥ ${computeBalance.amountAvailable}` : '--' }}</b></p>
            <div class="instance-picker">
              <button v-for="item in computeInstances" :key="item.instanceId" type="button" :class="{ selected: item.instanceId === computeConfiguration?.boundInstanceId }" :disabled="busy" @click="bindComputeInstance(item)">
                <span><b>{{ item.name ?? item.instanceId }}</b><small>{{ item.zone }} · {{ item.gpuType ? `RTX ${item.gpuType}` : '无 GPU 信息' }}</small></span><strong>{{ item.runningMode === 'noGpu' ? '无卡运行' : item.runningMode === 'gpu' ? 'GPU 运行' : item.rawState }}</strong>
              </button>
            </div>
          </template>
          <p v-if="computeNotice" class="connection-notice" :class="computeNoticeTone">{{ computeNotice }}</p>
        </div>
        <footer><span></span><span></span><button class="btn" type="button" :disabled="busy" @click="refreshCompute">刷新</button><button v-if="!computeConfigured" class="btn primary" type="button" :disabled="busy" @click="saveComputeCredentials">保存并验证</button><button v-else class="btn primary" type="button" @click="computeOpen=false">完成</button></footer>
      </section>
    </div>

    <div v-if="deepSeekOpen" class="connection-backdrop" role="presentation" @click.self="deepSeekOpen=false">
      <section class="connection-dialog" role="dialog" aria-modal="true" aria-labelledby="deepseek-title">
        <header><div><h2 id="deepseek-title">DeepSeek 内容规划</h2><p>资料会在点击“提取知识点”时发送给 DeepSeek；API Key 只保存在 Windows 凭据管理器。</p></div><button type="button" aria-label="关闭" @click="deepSeekOpen=false"><X :size="20"/></button></header>
        <div class="connection-form">
          <label><span>API 地址</span><input v-model="deepSeekForm.baseUrl" autocomplete="off"/></label>
          <label><span>模型</span><input v-model="deepSeekForm.model" autocomplete="off"/></label>
          <label><span>API Key</span><input v-model="deepSeekForm.apiKey" type="password" autocomplete="new-password" :placeholder="deepSeekConfigured ? '已保存；留空可只测试连接' : '输入 DeepSeek API Key'"/></label>
          <p v-if="deepSeekNotice" class="connection-notice" :class="deepSeekNoticeTone">{{ deepSeekNotice }}</p>
        </div>
        <footer><button v-if="deepSeekConfigured" class="btn link danger" type="button" :disabled="busy" @click="clearDeepSeek">移除密钥</button><span></span><button class="btn" type="button" @click="deepSeekOpen=false">取消</button><button class="btn primary" type="button" :disabled="busy" @click="saveOrTestDeepSeek">{{ deepSeekConfigured && !deepSeekForm.apiKey ? '测试连接' : '保存并验证' }}</button></footer>
      </section>
    </div>

  </section>
</template>

<script lang="ts">
import { Clock3 as ClockIcon, FileText as FileTextIcon } from "lucide-vue-next";
export default { components: { ClockIcon, FileTextIcon } };
</script>

<style scoped>
.settings-page{display:grid;grid-template-rows:78px 61px minmax(0,1fr);gap:0}.settings-head{display:flex;align-items:center;padding-left:16px}.settings-tabs{display:flex;gap:30px;border-bottom:1px solid var(--line);padding:0 20px}.settings-tabs button{height:61px;padding:0 12px;border:0;background:transparent;display:flex;align-items:center;gap:12px;font-size:17px;font-weight:700;color:#29446f;position:relative}.settings-tabs button.active{color:var(--blue)}.settings-tabs button.active:after{content:"";position:absolute;left:0;right:0;bottom:0;height:4px;border-radius:4px;background:var(--blue)}.settings-grid{min-height:0;padding-top:14px;display:grid;grid-template-columns:1.05fr 1fr 1fr;grid-template-rows:294px minmax(288px,1fr) 160px;gap:13px}.account{grid-column:1/3}.instance{grid-column:3/4}.connection{grid-column:1/4}.panel-head>span,.panel-title>span{display:flex;align-items:center;gap:8px;font-size:13px}.money-row{height:126px;margin:10px 18px 12px;display:grid;grid-template-columns:1fr .85fr 1.05fr;border-radius:9px;overflow:hidden;background:linear-gradient(110deg,#eaf3ff,#f7fbff)}.money-row article{display:flex;align-items:center;gap:16px;padding:16px 20px;border-right:1px solid #d7e2f0}.money-row svg{color:var(--blue)}.money-row article>div,.money-row article{color:#1b3159}.money-row span{display:block;font-size:14px}.money-row b{display:block;margin-top:8px;font-size:31px;color:#07183d}.money-row b small{font-size:13px}.money-row .cost{background:linear-gradient(120deg,#fff8ef,#fffaf5);color:#263d64}.money-row .cost svg,.money-row .cost b{color:#f26b0f}.money-row .cost small{display:block;margin-top:5px;color:#677b9d;font-size:11px}.account-actions{display:grid;grid-template-columns:1fr 1fr;gap:14px;padding:0 18px}.account-actions .btn{height:50px;font-size:17px}.instance-main{height:105px;display:grid;grid-template-columns:104px 1fr auto;align-items:center;padding:8px 18px;gap:14px}.server-art{width:100px;height:85px;display:grid;place-items:center;background:linear-gradient(145deg,#d7e7f7,#f8fbff);border-radius:8px;font-size:62px;color:#274b73}.instance-main h2{font-size:20px}.instance-main p{font-size:17px;margin:5px 0}.instance-main span{display:flex;align-items:center;gap:7px;font-size:14px}.instance-main>span{align-self:center}.instance-metrics{height:66px;display:grid;grid-template-columns:.8fr 1.25fr 1fr;border-top:1px solid #e1e8f1;padding:8px 16px}.instance-metrics>div{display:flex;flex-direction:column;border-right:1px solid #dfe6ef;padding-left:8px}.instance-metrics>div:last-child{border:0}.instance-metrics span{font-size:12px;color:#617596}.instance-metrics b{font-size:16px;margin-top:5px}.instance-actions{display:grid;grid-template-columns:.8fr 1fr 1.55fr;gap:9px;padding:6px 18px}.instance-actions .btn{min-height:40px;padding:0 8px}.panel-title{height:55px;padding:0 21px;display:flex;align-items:center;justify-content:space-between}.switch{width:49px;height:27px;border-radius:20px;background:#cbd6e6;position:relative}.switch:after{content:"";position:absolute;left:3px;top:3px;width:21px;height:21px;border-radius:50%;background:white;box-shadow:0 1px 4px #9aabc2}.switch.on{background:var(--blue)}.switch.on:after{left:25px}.setting-row{min-height:74px;margin:0 22px;border-bottom:1px solid #e0e7f1;display:flex;align-items:center;gap:15px}.setting-row svg{color:#16355f}.setting-row>div{display:flex;flex-direction:column;gap:6px;flex:1}.setting-row b{font-size:15px}.setting-row span{font-size:12px;color:#687c9e}.setting-row button{height:43px;min-width:150px;border:1px solid #cddbee;border-radius:7px;background:#fff}.check-list{padding:0 18px}.check-list p{height:33px;display:flex;align-items:center;gap:12px;border-bottom:1px solid #e7edf4;font-size:14px}.check-list p>span:last-child{margin-left:auto;display:flex;align-items:center;gap:7px}.service-icon{width:23px;height:23px;display:grid;place-items:center;color:#19407c;font-weight:800}.disk{height:57px;display:grid;grid-template-columns:30px 92px 1fr 120px 36px;align-items:center;gap:8px;padding:0 18px}.disk>span{font-size:12px;color:#5f7394}.warning-box{margin:12px 17px;padding:15px;border-radius:8px;background:#fff3e4;color:#ed6a0d;display:flex;flex-direction:column;gap:6px}.warning-box span{font-size:12px;color:#745d4e}.connection-cards{height:102px;display:grid;grid-template-columns:repeat(4,1fr);gap:10px;padding:12px 18px}.connection-cards article{border:1px solid #dbe4f0;border-radius:8px;display:grid;grid-template-columns:42px 1fr 15px;align-items:center;padding:0 13px;min-width:0}.connection-cards svg{color:var(--blue)}.connection-cards article>div{display:flex;flex-direction:column;gap:6px;min-width:0}.connection-cards span{font-size:12px;color:#687c9e;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.connection-cards .success-text{font-size:12px}.retention .setting-row{min-height:70px}@media(max-width:1380px){.settings-grid{grid-template-rows:270px minmax(260px,1fr) 145px}.settings-tabs{gap:14px}.money-row{height:112px}.money-row b{font-size:25px}.instance-main{height:92px}.server-art{width:80px;height:70px}.instance-actions{padding-left:10px;padding-right:10px}.connection-cards{height:88px}.connection-cards article{grid-template-columns:34px 1fr 12px;padding:0 9px}}
.waiting-text{color:#6e809d}
.settings-tabs button:disabled{opacity:.48;cursor:not-allowed}.connection-cards article[role="button"]{cursor:pointer;transition:.15s ease}.connection-cards article[role="button"]:hover,.connection-cards article[role="button"]:focus-visible{border-color:#8eb8f7;background:#f7faff;outline:0}.connection-backdrop{position:fixed;z-index:80;inset:30px 0 0 0;background:rgba(6,20,46,.32);display:grid;place-items:center;padding:24px}.connection-dialog{width:min(620px,calc(100vw - 80px));border:1px solid #cedbed;border-radius:13px;background:#fff;box-shadow:0 24px 70px rgba(16,45,88,.24);overflow:hidden}.connection-dialog>header{min-height:88px;padding:19px 22px;display:flex;align-items:flex-start;justify-content:space-between;border-bottom:1px solid var(--line);background:linear-gradient(135deg,#f7fbff,#fff)}.connection-dialog>header h2{font-size:22px}.connection-dialog>header p{margin-top:7px;color:#64799b;font-size:13px}.connection-dialog>header button{width:34px;height:34px;border:0;border-radius:7px;background:transparent;display:grid;place-items:center}.connection-dialog>header button:hover{background:#edf3fb}.connection-form{padding:20px 22px;display:flex;flex-direction:column;gap:16px}.connection-form label{display:grid;grid-template-columns:132px minmax(0,1fr);align-items:center;gap:8px 14px}.connection-form label>span{font-weight:700;color:#1b355f}.connection-form input{height:43px;border:1px solid #cbd9ec;border-radius:8px;padding:0 12px;background:#fff;user-select:text}.connection-form input:focus{outline:2px solid #cfe2ff;border-color:var(--blue)}.connection-form small{grid-column:2;color:#6d809f;font-size:12px}.connection-notice{margin:2px 0 0 146px;padding:10px 12px;border-radius:7px;background:#f1f5fa;color:#52698e;font-size:13px}.connection-notice.success{background:#e8f8f1;color:#087d51}.connection-notice.error{background:#fff0ee;color:#b33b35}.connection-dialog>footer{min-height:72px;padding:13px 22px;border-top:1px solid var(--line);display:grid;grid-template-columns:auto 1fr auto auto;align-items:center;gap:10px;background:#fbfdff}.connection-dialog button:disabled,.environment button:disabled{opacity:.55;cursor:not-allowed}.compute-summary{margin:0;padding:13px 15px;border-radius:8px;background:#eef6ff;color:#29486f}.compute-summary b{color:var(--blue);font-size:20px}.instance-picker{display:flex;flex-direction:column;gap:9px;max-height:280px;overflow:auto}.instance-picker>button{min-height:68px;padding:10px 14px;border:1px solid #d7e2ef;border-radius:8px;background:#fff;display:flex;align-items:center;justify-content:space-between;text-align:left;color:#17345f}.instance-picker>button.selected{border-color:var(--blue);background:#f1f7ff;box-shadow:0 0 0 1px var(--blue) inset}.instance-picker>button span{display:flex;flex-direction:column;gap:6px}.instance-picker>button small{color:#7183a0}.instance-picker>button strong{color:#315d9b;font-size:13px}
</style>
