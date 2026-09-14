<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { useRouter } from "vue-router";
import { ArrowRight, BookOpenText, Check, Clock3, Copy, Film, FolderOpen, Grid3X3, HardDrive, MoreVertical, Pencil, Plus, Search, ShieldCheck, SlidersHorizontal, Sparkles, Trash2, X } from "lucide-vue-next";
import { projectStates, type ZhihuaProject } from "../domain/projects";
import { useProjectStore } from "../stores/projects";
import { useStoryboardStore } from "../stores/storyboard";
import { storageRepository, type StorageUsage } from "../services/storageRepository";
import { readLocal, writeLocal } from "../services/nativeBridge";

const router = useRouter();
const store = useProjectStore();
const storyboard = useStoryboardStore();
const menuId = ref<string>();
const dialogMode = ref<"create" | "rename" | null>(null);
const createIntent = ref<"full" | "material" | "blank">("full");
const editingId = ref<string>();
const form = reactive({ title: "", audience: "", duration: "60" });
const formError = ref("");
const storageUsage = ref<StorageUsage>();
const stateTone: Record<string, string> = { "待导出": "orange", "已完成": "green", "生成中": "blue", "编排中": "blue", "草稿": "gray" };
const recentId = computed(() => [...store.items.value].sort((a, b) => (b.lastOpenedAt ?? "").localeCompare(a.lastOpenedAt ?? ""))[0]?.id);
onMounted(async () => {
  await Promise.all([
    store.load(),
    storageRepository.usage().then((value) => { storageUsage.value = value; }).catch(() => undefined),
  ]);
});

const recentProjects = computed(() => [...store.items.value]
  .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
  .slice(0, 3));
const usedDiskPercent = computed(() => {
  const usage = storageUsage.value;
  if (!usage?.totalBytes) return 0;
  return Math.max(0, Math.min(100, (usage.totalBytes - usage.availableBytes) / usage.totalBytes * 100));
});

function formatBytes(bytes?: number) {
  if (bytes == null) return "正在统计";
  if (bytes < 1024 ** 2) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

function formatDuration(seconds: number) {
  if (seconds < 60) return `${seconds} 秒`;
  return `${Math.floor(seconds / 60)} 分 ${String(seconds % 60).padStart(2, "0")} 秒`;
}
function formatEdited(value: string) {
  const date = new Date(value);
  if (date.toDateString() === new Date().toDateString()) return `今天 ${date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false })}`;
  return date.toLocaleDateString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit" }).replace(/\//g, "-");
}
const intentCopy = computed(() => ({
  full: { title: "制作完整视频", detail: "从资料、文字或一个主题开始，知画会引导你完成内容、分镜和成片。", action: "创建并进入内容" },
  material: { title: "快速生成素材", detail: "直接从提示词或参考图开始，自动准备第一个镜头，结果仍会完整保存。", action: "创建并进入分镜" },
  blank: { title: "空白项目", detail: "从空白分镜开始，自由安排自己的制作顺序。", action: "创建空白项目" },
})[createIntent.value]);

function showCreate(intent: "full" | "material" | "blank" = "full") {
  createIntent.value = intent;
  Object.assign(form, { title: "", audience: "", duration: "60" });
  formError.value = "";
  dialogMode.value = "create";
}
function showRename(project: ZhihuaProject) {
  form.title = project.title;
  editingId.value = project.id;
  formError.value = "";
  menuId.value = undefined;
  dialogMode.value = "rename";
}
async function submitDialog() {
  const title = form.title.trim();
  if (!title) { formError.value = "请输入项目名称"; return; }
  if (dialogMode.value === "create") {
    const project = await store.create({ title, audience: form.audience.trim(), targetDurationSeconds: Number(form.duration) || null });
    dialogMode.value = null;
    await store.open(project.id);
    if (createIntent.value === "material") {
      await storyboard.load(true);
      if (!storyboard.scenes.value.length) await storyboard.add();
      writeLocal(`zhihua.lastProjectRoute.${project.id}`, "/storyboard");
      await router.push("/storyboard");
    } else if (createIntent.value === "blank") {
      writeLocal(`zhihua.lastProjectRoute.${project.id}`, "/storyboard");
      await router.push("/storyboard");
    } else {
      writeLocal(`zhihua.lastProjectRoute.${project.id}`, "/sources");
      await router.push("/sources");
    }
  } else if (editingId.value) {
    await store.rename(editingId.value, title);
    dialogMode.value = null;
  }
}
async function openProject(id: string) {
  menuId.value = undefined;
  await store.open(id);
  const project = store.items.value.find((item) => item.id === id);
  const fallback = !project?.shotCount ? "/sources" : project.state === "待导出" || project.state === "已完成" ? "/export" : "/storyboard";
  const destination = readLocal<string>(`zhihua.lastProjectRoute.${id}`, fallback);
  await router.push(["/sources", "/storyboard", "/assets", "/export"].includes(destination) ? destination : fallback);
}

function nextAction(project: ZhihuaProject) {
  if (project.state === "已完成") return "查看成片";
  if (project.state === "待导出") return "检查并导出";
  if (project.state === "生成中") return "查看生成任务";
  if (project.shotCount) return `继续完善 ${project.shotCount} 个镜头`;
  return "导入内容或新增分镜";
}
async function duplicateProject(id: string) {
  menuId.value = undefined;
  await store.duplicate(id);
}
async function deleteProject(project: ZhihuaProject) {
  menuId.value = undefined;
  if (window.confirm(`确定删除项目“${project.title}”吗？将删除该项目的本地资料、素材、候选视频和导出文件，无法撤销。`)) await store.remove(project.id);
}
</script>

<template>
  <section class="page projects-page" :class="{ 'is-empty': !store.items.value.length }" @click="menuId = undefined">
    <header class="page-head">
      <div class="page-title-line"><h1>我的项目</h1><span class="subtitle">从资料或一个想法开始，知画会把过程和结果自动整理好</span></div>
      <div class="head-actions">
        <div class="workspace-promise"><ShieldCheck :size="20"/><div><strong>本地自动保存</strong><span>只有生成时才启动 GPU</span></div></div>
        <button v-if="store.items.value.length" class="btn primary" @click="showCreate('full')"><Plus :size="20"/>新建项目</button>
      </div>
    </header>

    <div v-if="store.items.value.length" class="project-toolbar">
      <label class="search-box"><Search :size="20"/><input v-model="store.query.value" placeholder="搜索项目（支持项目名、主题、受众）" /></label>
      <div class="tabs filters"><button v-for="item in projectStates" :key="item" class="tab-btn" :class="{active:store.status.value===item}" @click="store.status.value=item">{{ item }}</button></div>
      <button class="btn" :title="store.sortDescending.value ? '当前按最近编辑优先' : '当前按最早编辑优先'" @click="store.sortDescending.value=!store.sortDescending.value"><SlidersHorizontal :size="17"/>{{ store.sortDescending.value ? '最近编辑' : '最早编辑' }}</button>
    </div>

    <div v-if="store.visibleProjects.value.length" class="project-grid">
      <article v-for="project in store.visibleProjects.value" :key="project.id" class="project-card" :class="{selected:project.id===recentId}" tabindex="0" @click="openProject(project.id)" @keydown.enter="openProject(project.id)">
        <div class="project-cover" :class="`cover-${project.cover}`">
          <video v-if="project.coverUrl" :src="project.coverUrl" muted preload="metadata"></video>
          <div v-else-if="project.cover === 'empty'" class="empty-cover-state"><FolderOpen :size="34"/><span>尚未生成封面</span></div>
          <span v-if="project.id===recentId" class="recent">最近打开</span><span class="badge" :class="stateTone[project.state]">{{ project.state }}</span>
          <button class="cover-menu" title="项目操作" @click.stop="menuId = menuId===project.id ? undefined : project.id"><MoreVertical :size="20"/></button>
          <div v-if="menuId===project.id" class="project-menu" @click.stop><button @click="showRename(project)"><Pencil :size="15"/>重命名</button><button @click="duplicateProject(project.id)"><Copy :size="15"/>复制项目</button><button class="danger-text" @click="deleteProject(project)"><Trash2 :size="15"/>删除项目</button></div>
        </div>
        <div class="project-info"><h3>{{ project.title }}</h3><p class="ellipsis">{{ project.description }}</p><div class="project-meta"><span><Grid3X3 :size="17"/>{{ project.shotCount }} 个分镜</span><span><Clock3 :size="17"/>总时长 {{ formatDuration(project.durationSeconds) }}</span></div></div>
        <footer><span>最近编辑　{{ formatEdited(project.updatedAt) }}</span><span class="continue-label">{{ nextAction(project) }} <ArrowRight :size="14"/></span></footer>
      </article>
    </div>
    <div v-else-if="store.items.value.length" class="empty-projects"><Search :size="34"/><strong>没有符合条件的项目</strong><span>换个关键词或筛选条件试试</span></div>

    <section v-else class="start-workspace">
      <div class="start-copy">
        <span class="start-kicker"><Sparkles :size="16"/>从这里开始</span>
        <h2>你今天想完成什么？</h2>
        <p>两种方式都会自动创建项目并保存提示词、素材、候选和成片。以后可以随时继续，不必现在决定全部参数。</p>
      </div>
      <div class="start-options">
        <button class="start-card recommended" @click="showCreate('full')">
          <span class="start-icon"><BookOpenText :size="32"/></span>
          <i>推荐</i>
          <strong>制作完整视频</strong>
          <p>导入文档、粘贴文字或输入一个主题，先确认内容，再生成分镜与成片。</p>
          <b>从内容开始 <ArrowRight :size="17"/></b>
        </button>
        <button class="start-card" @click="showCreate('material')">
          <span class="start-icon purple"><Film :size="32"/></span>
          <strong>快速生成素材</strong>
          <p>直接上传参考图或填写画面描述，快速生成一张图片或一段视频。</p>
          <b>从一个镜头开始 <ArrowRight :size="17"/></b>
        </button>
      </div>
      <button class="blank-project" @click="showCreate('blank')"><Plus :size="16"/>创建空白项目</button>
      <div class="start-footnotes">
        <span><HardDrive :size="17"/>项目和生成结果保存在本机</span>
        <span><ShieldCheck :size="17"/>提交生成前会显示预计时间与费用</span>
      </div>
    </section>

    <div v-if="store.items.value.length" class="project-bottom">
      <div class="panel activity"><div class="panel-head"><h3>最近项目</h3><span>{{ recentProjects.length ? '按最近编辑排序' : '暂无记录' }}</span></div><div v-if="recentProjects.length" class="activity-list"><p v-for="project in recentProjects" :key="project.id" @click="openProject(project.id)"><span class="activity-icon blue">▤</span><b>{{ project.title }}</b><span>{{ project.state }} · {{ project.shotCount }} 个分镜</span><time>{{ formatEdited(project.updatedAt) }}</time></p></div><div v-else class="bottom-empty">创建项目后，最近编辑记录会显示在这里。</div></div>
      <div class="panel storage"><div class="panel-head"><h3>本地存储</h3><button class="btn link" @click="storageRepository.openProjectsRoot()">打开项目目录</button></div><div class="storage-body"><span class="drive">▰</span><div><div><strong>知画项目 {{ formatBytes(storageUsage?.projectBytes) }}</strong><span>磁盘可用 {{ formatBytes(storageUsage?.availableBytes) }}</span></div><div class="progress"><i :style="{width:`${usedDiskPercent}%`}"></i></div><p><span>● 项目文件 {{ formatBytes(storageUsage?.projectBytes) }}</span><span>● 数据库 {{ formatBytes(storageUsage?.databaseBytes) }}</span><span>● 所在磁盘已用 {{ usedDiskPercent.toFixed(0) }}%</span></p></div></div></div>
    </div>

    <div v-if="dialogMode" class="modal-backdrop" @click.self="dialogMode=null"><form class="project-dialog" @submit.prevent="submitDialog"><header><div><h2>{{ dialogMode==='create'?'新建项目':'重命名项目' }}</h2><p>{{ dialogMode==='create'?intentCopy.detail:'修改后的名称会自动保存。' }}</p></div><button type="button" @click="dialogMode=null"><X :size="20"/></button></header><div v-if="dialogMode==='create'" class="create-intents" aria-label="开始方式"><button type="button" :class="{active:createIntent==='full'}" @click="createIntent='full'"><BookOpenText :size="18"/><span><b>完整视频</b><small>推荐</small></span></button><button type="button" :class="{active:createIntent==='material'}" @click="createIntent='material'"><Film :size="18"/><span><b>快速素材</b><small>单个镜头</small></span></button><button type="button" :class="{active:createIntent==='blank'}" @click="createIntent='blank'"><Plus :size="18"/><span><b>空白项目</b><small>自由开始</small></span></button></div><label>项目名称<input v-model="form.title" maxlength="60" autofocus placeholder="例如：为什么会打雷？"/></label><template v-if="dialogMode==='create'"><div class="optional-label">以下内容可以稍后设置</div><div class="optional-fields"><label>目标受众<input v-model="form.audience" placeholder="例如：小学高年级"/></label><label>目标时长<select v-model="form.duration"><option value="30">约 30 秒</option><option value="60">约 60 秒</option><option value="90">约 90 秒</option><option value="">稍后设置</option></select></label></div></template><p v-if="formError" class="form-error">{{ formError }}</p><footer><button type="button" class="btn" @click="dialogMode=null">取消</button><button class="btn primary"><Check :size="17"/>{{ dialogMode==='create'?intentCopy.action:'保存名称' }}</button></footer></form></div>
  </section>
</template>

<style scoped>
.projects-page{display:flex;flex-direction:column;gap:10px}.page-head{min-height:70px}.compute{padding-top:4px}.local-projects{height:54px;min-width:310px;border:1px solid var(--line);border-radius:9px;background:#fff;display:flex;align-items:center;gap:12px;padding:0 17px;box-shadow:var(--shadow);color:var(--blue)}.local-projects div{display:flex;flex-direction:column;color:#203962}.local-projects span{font-size:13px;color:var(--muted);margin-top:2px}.project-toolbar{display:grid;grid-template-columns:minmax(320px,1.2fr) minmax(480px,1.45fr) auto auto;gap:12px;align-items:center}.filters{padding:5px;background:#fff;border:1px solid var(--line);border-radius:10px}.filters .tab-btn{border:0;min-width:72px}.project-grid{flex:1;min-height:0;display:grid;grid-template-columns:repeat(4,minmax(220px,1fr));grid-template-rows:repeat(2,minmax(0,1fr));gap:14px}.project-card{position:relative;min-height:0;display:grid;grid-template-rows:minmax(100px,1fr) auto 38px;background:#fff;border:1px solid var(--line);border-radius:10px;overflow:visible;box-shadow:var(--shadow);cursor:pointer}.project-card:focus-visible{outline:3px solid #9dc4ff}.project-card.selected{border:2px solid var(--blue);box-shadow:0 7px 20px rgba(15,111,255,.14)}.project-cover{position:relative;min-height:0;background-position:center;background-size:cover;border-radius:9px 9px 0 0}.project-cover .badge{position:absolute;top:10px;right:43px}.cover-menu{position:absolute;top:8px;right:8px;width:30px;height:34px;border:0;border-radius:8px;color:#fff;background:rgba(6,26,53,.62);display:grid;place-items:center}.project-menu{position:absolute;z-index:15;top:45px;right:8px;width:142px;padding:5px;border:1px solid #d4dfed;border-radius:8px;background:#fff;box-shadow:0 12px 28px rgba(16,43,84,.2)}.project-menu button{width:100%;height:34px;padding:0 10px;border:0;border-radius:6px;background:transparent;display:flex;align-items:center;gap:8px;color:#263e68}.project-menu button:hover{background:#edf4ff}.project-menu .danger-text{color:#dc3d46}.recent{position:absolute;z-index:2;top:8px;left:8px;border-radius:6px;padding:6px 10px;color:#fff;background:var(--blue);font-size:13px;font-weight:700}.project-info{padding:10px 14px 8px;overflow:hidden}.project-info h3{font-size:18px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.project-info p{margin-top:5px;color:#62779c;font-size:14px}.project-meta{display:flex;gap:22px;margin-top:10px;color:#263f6a;font-size:14px}.project-meta span{display:flex;align-items:center;gap:6px}.project-card>footer{border-top:1px solid #edf1f7;padding:0 14px;display:flex;align-items:center;justify-content:space-between;font-size:14px;color:#7386a7}.autosave{display:flex;align-items:center;gap:6px}.autosave i{width:18px;height:18px;border-radius:50%;display:grid;place-items:center;background:var(--blue);color:#fff;font-style:normal}.empty-projects{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:10px;color:#6b80a3}.empty-projects strong{font-size:19px;color:#29436e}.project-bottom{height:182px;display:grid;grid-template-columns:1.1fr 1fr;gap:14px}.activity-list{padding:5px 22px}.activity-list p{height:38px;display:flex;align-items:center;gap:12px;color:#263f6a;font-size:14px}.activity-list time{margin-left:auto;color:#6f82a3}.activity-icon{font-weight:800}.activity-icon.blue{color:var(--blue)}.activity-icon.purple{color:#8057e8}.storage-body{height:124px;display:flex;gap:18px;align-items:center;padding:12px 26px}.drive{width:62px;height:60px;display:grid;place-items:center;border-radius:10px;background:#edf5ff;color:var(--blue);font-size:38px}.storage-body>div{flex:1}.storage-body>div>div:first-child{display:flex;justify-content:space-between;margin-bottom:13px;color:#2b426b}.storage-body p{display:flex;gap:26px;margin-top:13px;color:#64789a;font-size:13px}.storage-body p span:nth-child(1){color:#2773e9}.storage-body p span:nth-child(2){color:#10a670}.modal-backdrop{position:fixed;z-index:100;inset:0;background:rgba(8,25,55,.38);display:grid;place-items:center}.project-dialog{width:470px;padding:22px;border:1px solid #d6e1ef;border-radius:14px;background:#fff;box-shadow:0 22px 60px rgba(14,40,78,.25)}.project-dialog header{display:flex;justify-content:space-between;margin-bottom:20px}.project-dialog header p{margin-top:7px;color:#7184a4;font-size:13px}.project-dialog header button{width:34px;height:34px;border:0;border-radius:7px;background:#eef3f9;display:grid;place-items:center}.project-dialog label{display:flex;flex-direction:column;gap:7px;margin-top:14px;font-size:14px;font-weight:700}.project-dialog input,.project-dialog select{height:44px;padding:0 12px;border:1px solid #cddbef;border-radius:8px;outline:none;background:#fff}.project-dialog input:focus,.project-dialog select:focus{border-color:var(--blue);box-shadow:0 0 0 3px #e5f0ff}.project-dialog>footer{display:flex;justify-content:flex-end;gap:10px;margin-top:22px}.form-error{margin-top:8px;color:#df3f48;font-size:13px}@media(max-width:1380px){.project-toolbar{grid-template-columns:280px 1fr auto auto}.filters .tab-btn{min-width:60px;padding:0 10px}.project-grid{gap:10px}.project-bottom{height:165px}.project-meta{gap:12px}.project-card>footer{padding:0 9px}}
.project-cover video{width:100%;height:100%;display:block;object-fit:cover}.cover-empty{background:linear-gradient(135deg,#dbeaff,#f7faff)}.activity-list p{cursor:pointer}.activity-list p>b{max-width:260px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.activity-list p>span:not(.activity-icon){color:#6f82a3;font-size:14px}.bottom-empty{height:120px;display:grid;place-items:center;color:var(--muted);font-size:13px}.panel-head>span{color:var(--muted);font-size:14px}
.empty-cover-state{position:absolute;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:8px;color:#7190bc}.empty-cover-state span{font-size:14px;font-weight:700}.create-intents{display:grid;grid-template-columns:repeat(3,1fr);gap:8px;margin:0 0 16px}.create-intents>button{height:62px;padding:0 11px;border:1px solid #d5e1f0;border-radius:10px;background:#f8faff;display:flex;align-items:center;gap:9px;text-align:left;color:#526b91}.create-intents>button.active{border-color:#5d9ffc;background:#edf5ff;color:#0869ef;box-shadow:inset 0 0 0 1px #9fc5fb}.create-intents span{display:flex;min-width:0;flex-direction:column;gap:3px}.create-intents b{font-size:13px;white-space:nowrap}.create-intents small{font-size:13px;color:#7a8eac;white-space:nowrap}
.workspace-promise{height:50px;padding:0 15px;border:1px solid var(--line);border-radius:10px;background:rgba(255,255,255,.88);display:flex;align-items:center;gap:10px;color:var(--blue)}.workspace-promise div{display:flex;flex-direction:column;gap:2px}.workspace-promise strong{font-size:13px;color:#17325f}.workspace-promise span{font-size:13px;color:var(--muted)}.project-toolbar{grid-template-columns:minmax(320px,1.2fr) minmax(480px,1.45fr) auto}.continue-label{display:flex;align-items:center;gap:5px;color:var(--blue);font-weight:700}.start-workspace{flex:1;min-height:0;display:flex;flex-direction:column;align-items:center;justify-content:center;padding-bottom:52px}.start-copy{text-align:center;max-width:720px}.start-kicker{display:inline-flex;align-items:center;gap:6px;padding:6px 10px;border-radius:99px;background:#eaf3ff;color:#0869ef;font-size:14px;font-weight:700}.start-copy h2{margin-top:13px;font-size:31px;letter-spacing:-.4px;color:#07183d}.start-copy p{margin-top:9px;color:#64789b;font-size:14px;line-height:1.7}.start-options{width:min(820px,78%);display:grid;grid-template-columns:1fr 1fr;gap:18px;margin-top:28px}.start-card{position:relative;min-height:238px;padding:25px;text-align:left;border:1px solid #d6e2f2;border-radius:16px;background:rgba(255,255,255,.92);box-shadow:0 12px 30px rgba(39,83,143,.09);display:flex;flex-direction:column;align-items:flex-start;transition:.18s ease}.start-card:hover{transform:translateY(-2px);border-color:#8bb8f7;box-shadow:0 15px 34px rgba(26,94,188,.14)}.start-card.recommended{border:2px solid #5d9ffc;background:linear-gradient(145deg,#fff,#f5f9ff)}.start-card>i{position:absolute;top:16px;right:16px;padding:4px 8px;border-radius:99px;background:#e9f3ff;color:#0869ef;font-size:13px;font-style:normal;font-weight:700}.start-icon{width:58px;height:58px;border-radius:14px;display:grid;place-items:center;color:#0869ef;background:#eaf3ff}.start-icon.purple{color:#7651d9;background:#f0ebff}.start-card>strong{margin-top:16px;font-size:20px;color:#10284f}.start-card>p{margin-top:7px;color:#65799d;font-size:13px;line-height:1.65}.start-card>b{margin-top:auto;display:flex;align-items:center;gap:7px;color:#0869ef;font-size:14px}.blank-project{margin-top:16px;border:0;background:transparent;color:#557097;display:flex;align-items:center;gap:6px}.blank-project:hover{color:var(--blue)}.start-footnotes{display:flex;gap:28px;margin-top:31px;color:#607594;font-size:14px}.start-footnotes span{display:flex;align-items:center;gap:7px}.project-dialog{width:520px;padding:24px}.project-dialog header p{max-width:405px;line-height:1.55}.optional-label{margin-top:18px;padding-top:14px;border-top:1px solid #e5ebf4;color:#7083a3;font-size:13px}.optional-fields{display:grid;grid-template-columns:1fr 1fr;gap:12px}.optional-fields label{margin-top:10px}@media(max-width:1380px){.project-toolbar{grid-template-columns:280px 1fr auto}.start-options{width:86%}.start-workspace{padding-bottom:16px}}
</style>
