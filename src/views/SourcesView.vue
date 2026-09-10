<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { AlertTriangle, Check, ChevronRight, Clock3, FileText, Pencil, Plus, Sparkles, Trash2, Upload, X } from "lucide-vue-next";
import type { KnowledgePoint, SourceDocument, SourceKind } from "../domain/sources";
import { useSourceStore } from "../stores/sources";
import { useRouter } from "vue-router";

const store = useSourceStore();
const router = useRouter();
const inputRef = ref<HTMLInputElement>();
const dragging = ref(false);
const pasteDialog = ref(false);
const notice = ref("");
const pasteForm = reactive({ name: "粘贴的文字", text: "" });
const filters: Array<"全部" | SourceKind> = ["全部", "PDF", "PPTX", "DOCX", "TXT", "IMAGE", "TEXT"];

const activeSources = computed(() => store.items.value.filter((item) => item.enabled && item.status === "ready"));
onMounted(store.load);

function kindCount(kind: "全部" | SourceKind) {
  return kind === "全部" ? store.items.value.length : store.items.value.filter((item) => item.kind === kind).length;
}
function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
}
function statusClass(source: SourceDocument) {
  return source.status === "ready" ? "ready" : source.status === "parsing" || source.status === "queued" ? "working" : "error";
}
function iconColor(kind: SourceKind) {
  return kind === "PDF" ? "red" : kind === "PPTX" ? "orange" : kind === "DOCX" ? "blue" : "slate";
}
function iconText(kind: SourceKind) {
  return kind === "PPTX" ? "P" : kind === "DOCX" ? "W" : kind === "IMAGE" ? "图" : kind === "TEXT" ? "文" : kind;
}
async function chooseFiles() {
  if (!(await store.importNative())) inputRef.value?.click();
}
async function importFiles(files: FileList | File[]) {
  const accepted = Array.from(files);
  if (!accepted.length) return;
  await store.importFiles(accepted);
  notice.value = `已导入并解析 ${accepted.length} 份资料`;
}
function onDrop(event: DragEvent) {
  dragging.value = false;
  if (event.dataTransfer?.files.length) importFiles(event.dataTransfer.files);
}
async function submitText() {
  if (!pasteForm.text.trim()) return;
  await store.pasteText(pasteForm.text.trim(), `${pasteForm.name.trim() || "粘贴的文字"}.txt`);
  pasteDialog.value = false;
  pasteForm.text = "";
  notice.value = "文字已导入并可用于知识点提取";
}
async function extractKnowledge() {
  if (!activeSources.value.length) { notice.value = "请先启用至少一份已解析完成的资料"; return; }
  try {
    notice.value = "DeepSeek 正在分析资料……";
    notice.value = await store.extractKnowledge() ? "知识点已从 DeepSeek 返回并保存" : "请在设置与算力中配置 DeepSeek API Key";
  } catch (error) {
    const value = error as { message?: string };
    notice.value = value?.message ?? String(error);
  }
}
function commitPoint(point: KnowledgePoint) {
  store.updatePoint(point.id, { title: point.title.trim(), detail: point.detail.trim(), confirmed: point.confirmed, needsConfirmation: point.needsConfirmation });
}
function focusPoint(id: string) {
  window.document.getElementById(`point-${id}`)?.focus();
}
async function createStoryboard() {
  if (store.unresolvedCount.value) return;
  try {
    notice.value = "DeepSeek 正在生成分镜初稿……";
    const count = await store.createStoryboard();
    if (!count) {
      notice.value = "桌面端分镜生成尚不可用";
      return;
    }
    await router.push("/storyboard");
  } catch (error) {
    const value = error as { message?: string };
    notice.value = value?.message ?? String(error);
  }
}
</script>

<template>
  <section class="page sources-page" @dragenter.prevent="dragging=true" @dragover.prevent @dragleave.self="dragging=false" @drop.prevent="onDrop">
    <header class="page-head">
      <div class="page-title-line"><h1>资料整理</h1><span class="subtitle">导入资料，核对原文与来源，再生成分镜脚本</span></div>
      <div class="head-actions"><div class="status-line compute"><span class="dot gray"></span><strong>优云智算</strong><b>无卡模式</b><span>|</span><span>资料整理不使用 GPU</span></div><button class="btn" @click="chooseFiles"><Upload :size="19"/>导入资料</button><button class="btn" @click="pasteDialog=true"><FileText :size="18"/>粘贴文字</button><button class="btn primary" @click="extractKnowledge"><Sparkles :size="19"/>提取知识点</button></div>
      <input ref="inputRef" class="hidden-input" type="file" multiple accept=".pdf,.pptx,.docx,.txt,image/*" @change="importFiles(($event.target as HTMLInputElement).files ?? [])"/>
    </header>

    <p v-if="notice" class="notice"><AlertTriangle :size="16"/>{{ notice }}<button @click="notice=''">×</button></p>

    <div class="sources-main">
      <aside class="panel file-panel">
        <div class="panel-head"><h3>资料列表（{{ store.items.value.length }}）</h3><button class="btn small" @click="chooseFiles"><Plus :size="16"/>导入</button></div>
        <div class="file-tabs"><button v-for="item in filters" :key="item" v-show="item==='全部'||kindCount(item)>0" :class="{active:store.filter.value===item}" @click="store.filter.value=item">{{ item }} {{ kindCount(item) }}</button></div>
        <div class="file-list">
          <article v-for="source in store.visibleSources.value" :key="source.id" :class="{selected:source.id===store.selected.value?.id,disabled:!source.enabled}" @click="store.selectedId.value=source.id">
            <span class="file-icon" :class="iconColor(source.kind)">{{ iconText(source.kind) }}</span>
            <div><strong>{{ source.name }}</strong><span>{{ formatSize(source.size) }}<template v-if="source.pageCount"> · {{ source.pageCount }} 页</template></span><em :class="statusClass(source)">● {{ source.statusMessage }}</em><i v-if="source.status==='parsing'" class="mini-progress"><b :style="{width:`${source.progress}%`}"></b></i></div>
            <div class="file-actions"><button class="toggle" :class="{on:source.enabled}" :aria-label="source.enabled?'停用资料':'启用资料'" @click.stop="store.toggle(source)"></button><button class="delete" title="删除资料" @click.stop="store.remove(source.id)"><Trash2 :size="15"/></button></div>
          </article>
          <div v-if="!store.visibleSources.value.length" class="empty-list">此分类还没有资料</div>
        </div>
      </aside>

      <section class="panel document-panel">
        <template v-if="store.selected.value"><div class="doc-toolbar"><span class="file-icon" :class="iconColor(store.selected.value.kind)">{{ iconText(store.selected.value.kind) }}</span><strong>{{ store.selected.value.name }}</strong><span class="divider"></span><span>{{ store.selected.value.statusMessage }}</span><button class="plain-action" @click="chooseFiles"><Upload :size="16"/>替换文件</button></div><div class="paper" :class="{pending:store.selected.value.status!=='ready'}"><h2>{{ store.selected.value.name }}</h2><p v-for="(paragraph,index) in store.selected.value.extractedText.split(/\n+/).filter(Boolean)" :key="index" :class="{highlight:index===2}">{{ paragraph }}</p><div v-if="!store.selected.value.extractedText" class="no-preview"><FileText :size="40"/><strong>等待提取文本</strong><span>解析完成后可在这里查看和核对原文。</span></div></div></template>
        <div v-else class="no-preview full"><Upload :size="44"/><strong>导入第一份资料</strong><span>支持 PDF、PPTX、DOCX、TXT 和常见图片。</span><button class="btn primary" @click="chooseFiles">选择文件</button></div>
      </section>

      <aside class="panel insight-panel">
        <div class="ai-head"><span class="deepseek">鲸</span><span class="dot"></span><div><strong>DeepSeek 内容规划</strong><small>{{ store.enabledReadyCount.value }} 份资料可用于提取</small></div><button @click="extractKnowledge">重新提取</button></div>
        <div class="insight-scroll">
          <div class="two-fields"><label>目标受众<span>小学高年级　⌄</span></label><label>建议时长<span><Clock3 :size="17"/>42 秒　⌄</span></label></div>
          <div class="subhead"><h3>核心知识点（{{ store.points.value.length }}）</h3><button @click="store.addPoint"><Plus :size="15"/>自定义添加</button></div>
          <div class="point-list"><button v-for="(point,index) in store.points.value" :key="point.id" :class="{warn:point.needsConfirmation&&!point.confirmed}" @click="focusPoint(point.id)"><b>{{ index+1 }}</b><span>{{ point.title }}</span><i>{{ point.needsConfirmation&&!point.confirmed?'!':'✓' }}</i></button></div>
          <div class="subhead"><h3>来源引用</h3><span>{{ activeSources.length }} 份已启用资料</span></div>
          <p v-for="source in activeSources.slice(0,4)" :key="source.id" class="citation" :class="{active:source.id===store.selected.value?.id}" @click="store.selectedId.value=source.id">{{ source.name }}　{{ source.pageCount ? `${source.pageCount} 页` : '全文' }}</p>
        </div>
        <button class="btn primary generate" :disabled="store.unresolvedCount.value>0 || !store.points.value.length" @click="createStoryboard">确认内容并生成分镜 <ChevronRight :size="20"/></button>
        <p v-if="store.unresolvedCount.value" class="confirm-hint">还有 {{ store.unresolvedCount.value }} 条内容需要确认</p>
      </aside>
    </div>

    <div class="knowledge-panel panel">
      <div class="panel-head"><h3>提取的知识点（{{ store.points.value.length }}）</h3><span>标题、说明和确认状态均可直接编辑</span></div>
      <div class="knowledge-grid"><article v-for="(point,index) in store.points.value" :key="point.id" :class="{warn:point.needsConfirmation&&!point.confirmed}"><b>{{ index+1 }}</b><input :id="`point-${point.id}`" v-model="point.title" aria-label="知识点标题" @change="commitPoint(point)"/><textarea v-model="point.detail" aria-label="知识点说明" @change="commitPoint(point)"></textarea><footer><span>{{ point.sourceRefs[0] ? `来源：${point.sourceRefs[0].sourceName} · ${point.sourceRefs[0].location}` : '缺少来源' }}</span><label><input v-model="point.confirmed" type="checkbox" @change="commitPoint(point)"/><Check :size="14"/>已确认</label></footer></article><button class="add-point" @click="store.addPoint"><Plus :size="22"/>添加知识点</button></div>
    </div>

    <div v-if="dragging" class="drop-overlay"><Upload :size="46"/><strong>松开即可导入资料</strong><span>支持 PDF、PPTX、DOCX、TXT 和常见图片</span></div>
    <div v-if="pasteDialog" class="modal-backdrop" @click.self="pasteDialog=false"><form class="paste-dialog" @submit.prevent="submitText"><header><div><h2>粘贴文字资料</h2><p>纯文本会直接保存为可用来源。</p></div><button type="button" @click="pasteDialog=false"><X :size="20"/></button></header><label>资料名称<input v-model="pasteForm.name" maxlength="60"/></label><label>文字内容<textarea v-model="pasteForm.text" autofocus placeholder="在这里粘贴课程资料、讲稿或事实说明……"></textarea></label><footer><button type="button" class="btn" @click="pasteDialog=false">取消</button><button class="btn primary">导入文字</button></footer></form></div>
  </section>
</template>

<style scoped>
.sources-page{display:grid;grid-template-rows:80px auto minmax(0,1fr) 254px;gap:10px}.sources-page:not(:has(.notice)){grid-template-rows:80px minmax(0,1fr) 254px}.compute{height:48px;padding:0 14px;border:1px solid var(--line);border-radius:9px;background:#fff}.hidden-input{display:none}.notice{min-height:34px;margin:0 2px;padding:7px 10px;border:1px solid #ffd08a;border-radius:7px;display:flex;align-items:center;gap:7px;background:#fff7e9;color:#9b5a10;font-size:13px}.notice button{margin-left:auto;border:0;background:transparent;color:inherit;font-size:19px}.sources-main{min-height:0;display:grid;grid-template-columns:350px minmax(440px,1fr) 396px;gap:12px}.file-panel,.document-panel,.insight-panel{min-height:0;overflow:hidden}.file-tabs{display:flex;gap:5px;padding:9px 10px;overflow-x:auto}.file-tabs button{height:33px;border:0;border-radius:6px;padding:0 10px;white-space:nowrap;background:#f1f5fb;color:#53688e}.file-tabs .active{color:white;background:var(--blue)}.file-list{height:calc(100% - 90px);padding:0 8px;overflow-y:auto}.file-list article{position:relative;min-height:88px;border-bottom:1px solid #e9eef6;display:grid;grid-template-columns:46px 1fr 32px;align-items:center;gap:9px;padding:7px 8px;cursor:pointer}.file-list article.selected{border:2px solid var(--blue);border-radius:9px;background:#f4f8ff}.file-list article.disabled{opacity:.58}.file-icon{width:31px;height:40px;border-radius:4px;display:grid;place-items:center;color:#fff;font-weight:800;font-size:11px;flex:0 0 auto}.file-icon.red{background:#e63746}.file-icon.orange{background:#f06c23}.file-icon.blue{background:#2874da}.file-icon.slate{background:#667995}.file-list article>div:nth-child(2){display:flex;flex-direction:column;gap:4px;min-width:0}.file-list strong{font-size:14px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.file-list span,.file-list em{font-size:11px;color:#687da0}.file-list em{font-style:normal}.file-list .ready{color:#0a9d62}.file-list .working{color:#2c72d7}.file-list .error{color:#d84850}.file-actions{display:flex;flex-direction:column;gap:7px;align-items:center}.toggle{width:34px;height:19px;border:0;border-radius:20px;background:#ced8e7;position:relative}.toggle:after{content:"";position:absolute;width:15px;height:15px;left:2px;top:2px;border-radius:50%;background:#fff}.toggle.on{background:var(--blue)}.toggle.on:after{left:17px}.delete{width:27px;height:25px;border:0;border-radius:5px;display:grid;place-items:center;background:transparent;color:#9aabc3}.delete:hover{background:#fff0f0;color:#dd3d46}.mini-progress{height:3px;border-radius:3px;background:#dbe6f5;overflow:hidden}.mini-progress b{display:block;height:100%;background:var(--blue)}.empty-list{padding:40px 12px;text-align:center;color:#8394af}.doc-toolbar{height:54px;border-bottom:1px solid var(--line);display:flex;align-items:center;gap:11px;padding:0 14px;color:#51678c}.doc-toolbar .file-icon{width:25px;height:31px}.doc-toolbar strong{font-size:15px;color:#1a315b;max-width:42%;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.divider{height:24px;width:1px;background:#dfe6f1;margin:0 5px}.plain-action{margin-left:auto;border:0;background:transparent;color:var(--blue);display:flex;align-items:center;gap:5px}.paper{height:calc(100% - 54px);padding:30px 36px;overflow-y:auto;background:linear-gradient(90deg,#fff,#fcfdff);user-select:text}.paper.pending{background:#fafcff}.paper h2{font-size:23px;margin-bottom:20px}.paper p{font-size:17px;line-height:1.85;margin-bottom:19px;color:#213c6a;white-space:pre-wrap}.paper .highlight{background:#dceeff;box-shadow:0 0 0 4px #dceeff;border-radius:2px}.no-preview{height:80%;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:9px;color:#7a8dac}.no-preview strong{font-size:18px;color:#2c4771}.no-preview.full{height:100%}.insight-panel{display:flex;flex-direction:column}.ai-head{min-height:66px;border-bottom:1px solid var(--line);display:flex;align-items:center;gap:9px;padding:0 14px}.deepseek{font-size:30px;color:#1769ef}.ai-head>div{display:flex;flex-direction:column;gap:3px}.ai-head small{color:#7385a4}.ai-head>button{margin-left:auto;border:1px solid #d4dfef;border-radius:7px;background:#fff;height:34px}.insight-scroll{flex:1;min-height:0;overflow:auto;padding:14px}.two-fields{display:grid;grid-template-columns:1fr 1fr;gap:12px}.two-fields label{font-size:13px;font-weight:700}.two-fields span{height:45px;margin-top:6px;border:1px solid #d4dfef;border-radius:7px;display:flex;align-items:center;gap:6px;padding:0 10px;font-weight:600}.subhead{display:flex;align-items:center;justify-content:space-between;margin:15px 0 8px}.subhead button{height:31px;border:1px solid #d5e1f2;border-radius:6px;background:white;color:var(--blue)}.subhead>span{color:#7184a4;font-size:12px}.point-list{display:flex;flex-direction:column;gap:6px}.point-list>button{min-height:40px;border:1px solid #e0e7f1;border-radius:7px;display:grid;grid-template-columns:26px 1fr 22px;align-items:center;padding:5px 9px;background:#fff;text-align:left;font-size:13px}.point-list b{width:22px;height:22px;border-radius:50%;display:grid;place-items:center;background:#eaf3ff;color:var(--blue)}.point-list i{font-style:normal;color:#6680a5}.point-list .warn{border-color:#ffbf82;background:#fff7ed;color:#d95d00}.citation{font-size:12px;padding:7px 9px;color:#5b6f92;cursor:pointer}.citation.active{border-left:3px solid var(--blue);color:var(--blue);background:#f0f6ff}.generate{margin:8px 14px 4px;min-height:50px;font-size:16px}.generate:disabled{cursor:not-allowed;opacity:.48}.confirm-hint{text-align:center;margin:2px 0 8px;color:#d46a16;font-size:12px}.knowledge-panel{overflow:hidden}.knowledge-panel>.panel-head>span{font-size:12px;color:#7184a4}.knowledge-grid{height:calc(100% - 46px);display:grid;grid-auto-flow:column;grid-auto-columns:minmax(220px,1fr);gap:8px;padding:8px 10px 12px;overflow-x:auto}.knowledge-grid article{height:185px;border:1px solid #d8e3f2;border-radius:8px;padding:10px;display:grid;grid-template-columns:27px 1fr;grid-template-rows:29px 1fr 35px;gap:5px}.knowledge-grid article>b{width:24px;height:24px;border-radius:8px;display:grid;place-items:center;background:#eaf3ff;color:var(--blue)}.knowledge-grid article>input{min-width:0;border:0;border-bottom:1px solid transparent;outline:none;font-weight:700;color:#203a65;background:transparent}.knowledge-grid article>input:focus{border-color:var(--blue)}.knowledge-grid article textarea{grid-column:1/3;resize:none;border:1px solid transparent;border-radius:5px;padding:5px;background:transparent;color:#617697;font-size:12px;line-height:1.45}.knowledge-grid article textarea:focus{outline:none;border-color:#b8d4ff;background:#fff}.knowledge-grid article footer{grid-column:1/3;display:flex;flex-direction:column;gap:3px;font-size:10px;color:#6880a3;overflow:hidden}.knowledge-grid article footer span{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.knowledge-grid article footer label{display:flex;align-items:center;gap:4px;color:#24724d}.knowledge-grid article.warn{border-color:#ffae6e;background:#fffaf4}.add-point{height:185px;border:1px dashed #a8bcd9;border-radius:8px;background:#f8fbff;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:8px;color:var(--blue)}.small{min-height:32px;padding:0 10px}.drop-overlay{position:fixed;z-index:90;inset:52px 24px 24px 174px;border:3px dashed #58a0ff;border-radius:18px;background:rgba(239,247,255,.94);display:flex;flex-direction:column;align-items:center;justify-content:center;gap:12px;color:#1b6de3}.drop-overlay strong{font-size:24px}.modal-backdrop{position:fixed;z-index:100;inset:0;background:rgba(8,25,55,.38);display:grid;place-items:center}.paste-dialog{width:560px;padding:22px;border:1px solid #d6e1ef;border-radius:14px;background:#fff;box-shadow:0 22px 60px rgba(14,40,78,.25)}.paste-dialog header{display:flex;justify-content:space-between;margin-bottom:16px}.paste-dialog header p{margin-top:6px;color:#7184a4;font-size:13px}.paste-dialog header button{width:34px;height:34px;border:0;border-radius:7px;background:#eef3f9}.paste-dialog label{display:flex;flex-direction:column;gap:7px;margin-top:12px;font-size:14px;font-weight:700}.paste-dialog input,.paste-dialog textarea{border:1px solid #cddbef;border-radius:8px;padding:10px;outline:none}.paste-dialog input{height:43px}.paste-dialog textarea{height:220px;resize:vertical;line-height:1.6}.paste-dialog input:focus,.paste-dialog textarea:focus{border-color:var(--blue);box-shadow:0 0 0 3px #e5f0ff}.paste-dialog footer{display:flex;justify-content:flex-end;gap:10px;margin-top:18px}@media(max-width:1380px){.sources-main{grid-template-columns:300px minmax(390px,1fr) 350px}.paper{padding:25px}.paper p{font-size:15px}.knowledge-panel{height:220px}.sources-page,.sources-page:not(:has(.notice)){grid-template-rows:74px minmax(0,1fr) 220px}.sources-page:has(.notice){grid-template-rows:74px auto minmax(0,1fr) 220px}.head-actions{gap:6px}.head-actions .btn{padding:0 9px}}
</style>
