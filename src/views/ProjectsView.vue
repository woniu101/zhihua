<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { useRouter } from "vue-router";
import { Check, Clock3, Copy, FolderOpen, Grid3X3, MoreVertical, Pencil, Plus, Search, SlidersHorizontal, Trash2, X } from "lucide-vue-next";
import { projectStates, type ZhihuaProject } from "../domain/projects";
import { useProjectStore } from "../stores/projects";

const router = useRouter();
const store = useProjectStore();
const menuId = ref<string>();
const dialogMode = ref<"create" | "rename" | null>(null);
const editingId = ref<string>();
const form = reactive({ title: "", audience: "", duration: "60" });
const formError = ref("");
const stateTone: Record<string, string> = { "待导出": "orange", "已完成": "green", "生成中": "blue", "编排中": "blue", "草稿": "gray" };
const recentId = computed(() => [...store.items.value].sort((a, b) => (b.lastOpenedAt ?? "").localeCompare(a.lastOpenedAt ?? ""))[0]?.id);
onMounted(store.load);

function formatDuration(seconds: number) {
  if (seconds < 60) return `${seconds} 秒`;
  return `${Math.floor(seconds / 60)} 分 ${String(seconds % 60).padStart(2, "0")} 秒`;
}
function formatEdited(value: string) {
  const date = new Date(value);
  if (date.toDateString() === new Date().toDateString()) return `今天 ${date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false })}`;
  return date.toLocaleDateString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit" }).replace(/\//g, "-");
}
function showCreate() {
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
    await openProject(project.id);
  } else if (editingId.value) {
    await store.rename(editingId.value, title);
    dialogMode.value = null;
  }
}
async function openProject(id: string) {
  menuId.value = undefined;
  await store.open(id);
  await router.push("/sources");
}
async function duplicateProject(id: string) {
  menuId.value = undefined;
  await store.duplicate(id);
}
async function deleteProject(project: ZhihuaProject) {
  menuId.value = undefined;
  if (window.confirm(`确定删除项目“${project.title}”吗？此操作只删除当前本地项目记录。`)) await store.remove(project.id);
}
</script>

<template>
  <section class="page projects-page" @click="menuId = undefined">
    <header class="page-head">
      <div class="page-title-line"><h1>项目</h1><span class="subtitle">管理课程与科普视频项目</span></div>
      <div class="head-actions">
        <div class="status-line compute"><span class="dot gray"></span><strong>优云智算</strong><b>无卡模式</b><span>|</span><span>需要生成时再切换 GPU</span></div>
        <div class="local-projects"><FolderOpen :size="24"/><div><strong>本地项目</strong><span>{{ store.items.value.length }} 个 · 已自动保存</span></div></div>
      </div>
    </header>

    <div class="project-toolbar">
      <label class="search-box"><Search :size="20"/><input v-model="store.query.value" placeholder="搜索项目（支持项目名、主题、受众）" /></label>
      <div class="tabs filters"><button v-for="item in projectStates" :key="item" class="tab-btn" :class="{active:store.status.value===item}" @click="store.status.value=item">{{ item }}</button></div>
      <button class="btn" :title="store.sortDescending.value ? '当前按最近编辑优先' : '当前按最早编辑优先'" @click="store.sortDescending.value=!store.sortDescending.value"><SlidersHorizontal :size="17"/>{{ store.sortDescending.value ? '最近编辑' : '最早编辑' }}</button>
      <button class="btn primary" @click="showCreate"><Plus :size="20"/>新建项目</button>
    </div>

    <div v-if="store.visibleProjects.value.length" class="project-grid">
      <article v-for="project in store.visibleProjects.value" :key="project.id" class="project-card" :class="{selected:project.id===recentId}" tabindex="0" @click="openProject(project.id)" @keydown.enter="openProject(project.id)">
        <div class="project-cover" :class="`cover-${project.cover}`">
          <span v-if="project.id===recentId" class="recent">最近打开</span><span class="badge" :class="stateTone[project.state]">{{ project.state }}</span>
          <button class="cover-menu" title="项目操作" @click.stop="menuId = menuId===project.id ? undefined : project.id"><MoreVertical :size="20"/></button>
          <div v-if="menuId===project.id" class="project-menu" @click.stop><button @click="showRename(project)"><Pencil :size="15"/>重命名</button><button @click="duplicateProject(project.id)"><Copy :size="15"/>复制项目</button><button class="danger-text" @click="deleteProject(project)"><Trash2 :size="15"/>删除项目</button></div>
        </div>
        <div class="project-info"><h3>{{ project.title }}</h3><p class="ellipsis">{{ project.description }}</p><div class="project-meta"><span><Grid3X3 :size="17"/>{{ project.shotCount }} 个分镜</span><span><Clock3 :size="17"/>总时长 {{ formatDuration(project.durationSeconds) }}</span></div></div>
        <footer><span>最近编辑　{{ formatEdited(project.updatedAt) }}</span><span class="autosave"><i>✓</i>已自动保存</span></footer>
      </article>
    </div>
    <div v-else class="empty-projects"><Search :size="34"/><strong>没有符合条件的项目</strong><span>换个关键词或筛选条件试试</span></div>

    <div class="project-bottom">
      <div class="panel activity"><div class="panel-head"><h3>最近活动</h3><button class="btn link">查看全部</button></div><div class="activity-list"><p><span class="activity-icon blue">▤</span>项目修改会立即保存到本机<time>自动保存</time></p><p><span class="activity-icon blue">■</span>打开项目后继续整理资料和知识点<time>当前流程</time></p><p><span class="activity-icon purple">●</span>GPU 仅在生成任务需要时启动<time>无卡待机</time></p></div></div>
      <div class="panel storage"><div class="panel-head"><h3>本地存储</h3><button class="btn link">管理本地文件</button></div><div class="storage-body"><span class="drive">▰</span><div><div><strong>项目文件 42.6 GB</strong><span>剩余空间 57.4 GB</span></div><div class="progress"><i style="width:62%"></i></div><p><span>● 项目文件 42.6 GB</span><span>● 素材文件 9.4 GB</span><span>● 其他 0 GB</span></p></div></div></div>
    </div>

    <div v-if="dialogMode" class="modal-backdrop" @click.self="dialogMode=null"><form class="project-dialog" @submit.prevent="submitDialog"><header><div><h2>{{ dialogMode==='create'?'新建项目':'重命名项目' }}</h2><p>{{ dialogMode==='create'?'只需填写名称即可开始，其他内容可稍后补充。':'修改后的名称会自动保存。' }}</p></div><button type="button" @click="dialogMode=null"><X :size="20"/></button></header><label>项目名称<input v-model="form.title" maxlength="60" autofocus placeholder="例如：为什么会打雷？"/></label><template v-if="dialogMode==='create'"><label>目标受众（可选）<input v-model="form.audience" placeholder="例如：小学高年级"/></label><label>目标时长（可选）<select v-model="form.duration"><option value="30">约 30 秒</option><option value="60">约 60 秒</option><option value="90">约 90 秒</option><option value="">稍后设置</option></select></label></template><p v-if="formError" class="form-error">{{ formError }}</p><footer><button type="button" class="btn" @click="dialogMode=null">取消</button><button class="btn primary"><Check :size="17"/>{{ dialogMode==='create'?'创建并打开':'保存名称' }}</button></footer></form></div>
  </section>
</template>

<style scoped>
.projects-page{display:flex;flex-direction:column;gap:10px}.page-head{min-height:70px}.compute{padding-top:4px}.local-projects{height:54px;min-width:310px;border:1px solid var(--line);border-radius:9px;background:#fff;display:flex;align-items:center;gap:12px;padding:0 17px;box-shadow:var(--shadow);color:var(--blue)}.local-projects div{display:flex;flex-direction:column;color:#203962}.local-projects span{font-size:13px;color:var(--muted);margin-top:2px}.project-toolbar{display:grid;grid-template-columns:minmax(320px,1.2fr) minmax(480px,1.45fr) auto auto;gap:12px;align-items:center}.filters{padding:5px;background:#fff;border:1px solid var(--line);border-radius:10px}.filters .tab-btn{border:0;min-width:72px}.project-grid{flex:1;min-height:0;display:grid;grid-template-columns:repeat(4,minmax(220px,1fr));grid-template-rows:repeat(2,minmax(0,1fr));gap:14px}.project-card{position:relative;min-height:0;display:grid;grid-template-rows:minmax(100px,1fr) auto 38px;background:#fff;border:1px solid var(--line);border-radius:10px;overflow:visible;box-shadow:var(--shadow);cursor:pointer}.project-card:focus-visible{outline:3px solid #9dc4ff}.project-card.selected{border:2px solid var(--blue);box-shadow:0 7px 20px rgba(15,111,255,.14)}.project-cover{position:relative;min-height:0;background-position:center;background-size:cover;border-radius:9px 9px 0 0}.project-cover .badge{position:absolute;top:10px;right:43px}.cover-menu{position:absolute;top:8px;right:8px;width:30px;height:34px;border:0;border-radius:8px;color:#fff;background:rgba(6,26,53,.62);display:grid;place-items:center}.project-menu{position:absolute;z-index:15;top:45px;right:8px;width:142px;padding:5px;border:1px solid #d4dfed;border-radius:8px;background:#fff;box-shadow:0 12px 28px rgba(16,43,84,.2)}.project-menu button{width:100%;height:34px;padding:0 10px;border:0;border-radius:6px;background:transparent;display:flex;align-items:center;gap:8px;color:#263e68}.project-menu button:hover{background:#edf4ff}.project-menu .danger-text{color:#dc3d46}.recent{position:absolute;z-index:2;top:8px;left:8px;border-radius:6px;padding:6px 10px;color:#fff;background:var(--blue);font-size:13px;font-weight:700}.project-info{padding:10px 14px 8px;overflow:hidden}.project-info h3{font-size:18px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.project-info p{margin-top:5px;color:#62779c;font-size:14px}.project-meta{display:flex;gap:22px;margin-top:10px;color:#263f6a;font-size:14px}.project-meta span{display:flex;align-items:center;gap:6px}.project-card>footer{border-top:1px solid #edf1f7;padding:0 14px;display:flex;align-items:center;justify-content:space-between;font-size:12px;color:#7386a7}.autosave{display:flex;align-items:center;gap:6px}.autosave i{width:18px;height:18px;border-radius:50%;display:grid;place-items:center;background:var(--blue);color:#fff;font-style:normal}.empty-projects{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:10px;color:#6b80a3}.empty-projects strong{font-size:19px;color:#29436e}.project-bottom{height:182px;display:grid;grid-template-columns:1.1fr 1fr;gap:14px}.activity-list{padding:5px 22px}.activity-list p{height:38px;display:flex;align-items:center;gap:12px;color:#263f6a;font-size:14px}.activity-list time{margin-left:auto;color:#6f82a3}.activity-icon{font-weight:800}.activity-icon.blue{color:var(--blue)}.activity-icon.purple{color:#8057e8}.storage-body{height:124px;display:flex;gap:18px;align-items:center;padding:12px 26px}.drive{width:62px;height:60px;display:grid;place-items:center;border-radius:10px;background:#edf5ff;color:var(--blue);font-size:38px}.storage-body>div{flex:1}.storage-body>div>div:first-child{display:flex;justify-content:space-between;margin-bottom:13px;color:#2b426b}.storage-body p{display:flex;gap:26px;margin-top:13px;color:#64789a;font-size:13px}.storage-body p span:nth-child(1){color:#2773e9}.storage-body p span:nth-child(2){color:#10a670}.modal-backdrop{position:fixed;z-index:100;inset:0;background:rgba(8,25,55,.38);display:grid;place-items:center}.project-dialog{width:470px;padding:22px;border:1px solid #d6e1ef;border-radius:14px;background:#fff;box-shadow:0 22px 60px rgba(14,40,78,.25)}.project-dialog header{display:flex;justify-content:space-between;margin-bottom:20px}.project-dialog header p{margin-top:7px;color:#7184a4;font-size:13px}.project-dialog header button{width:34px;height:34px;border:0;border-radius:7px;background:#eef3f9;display:grid;place-items:center}.project-dialog label{display:flex;flex-direction:column;gap:7px;margin-top:14px;font-size:14px;font-weight:700}.project-dialog input,.project-dialog select{height:44px;padding:0 12px;border:1px solid #cddbef;border-radius:8px;outline:none;background:#fff}.project-dialog input:focus,.project-dialog select:focus{border-color:var(--blue);box-shadow:0 0 0 3px #e5f0ff}.project-dialog>footer{display:flex;justify-content:flex-end;gap:10px;margin-top:22px}.form-error{margin-top:8px;color:#df3f48;font-size:13px}@media(max-width:1380px){.project-toolbar{grid-template-columns:280px 1fr auto auto}.filters .tab-btn{min-width:60px;padding:0 10px}.project-grid{gap:10px}.project-bottom{height:165px}.project-meta{gap:12px}.project-card>footer{padding:0 9px}}
</style>
