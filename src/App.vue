<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRoute } from "vue-router";
import {
  FolderKanban,
  HelpCircle,
  Minus,
  Settings,
  Square,
  X,
} from "lucide-vue-next";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import TaskCenter from "./components/TaskCenter.vue";
import ProjectWorkspaceBar from "./components/ProjectWorkspaceBar.vue";
import BrandLogo from "./components/BrandLogo.vue";
import ComputeStatus from "./components/ComputeStatus.vue";

const route = useRoute();
const appWindow = isTauri() ? getCurrentWindow() : null;
const isSettings = computed(() => route.path === "/settings");
const projectRoutes = ["/sources", "/storyboard", "/assets", "/export"];
const isProjectWorkspace = computed(() => projectRoutes.includes(route.path));
const helpOpen = ref(false);
const closing = ref(false);
let unlistenClose: (() => void) | undefined;

const minimizeWindow = () => appWindow?.minimize();
const toggleMaximizeWindow = () => appWindow?.toggleMaximize();
const beginWindowDrag = (event: MouseEvent) => {
  if (!appWindow || event.button !== 0) return;
  void appWindow.startDragging();
};

async function closeWindow() {
  if (!appWindow || closing.value) return;
  closing.value = true;
  try {
    await Promise.race([
      invoke("prepare_application_exit"),
      new Promise((_, reject) => window.setTimeout(() => reject(new Error("退出保护检查超时")), 8_000)),
    ]);
  } catch (error) {
    console.info("[知画] 退出保护由平台定时关机继续接管。", error);
    const leaveAnyway = window.confirm("暂时无法确认 GPU 已关闭或平台定时关机已生效。继续退出可能产生额外费用。\n\n仍要退出知画吗？");
    if (!leaveAnyway) {
      closing.value = false;
      return;
    }
  }
  await appWindow.destroy();
}

onMounted(async () => {
  if (!appWindow) return;
  unlistenClose = await appWindow.onCloseRequested((event) => {
    event.preventDefault();
    void closeWindow();
  });
});

onBeforeUnmount(() => unlistenClose?.());

const nav = [
  { path: "/projects", label: "项目", icon: FolderKanban },
];
</script>

<template>
  <div class="app-frame">
    <header class="appbar">
      <RouterLink class="app-brand" to="/projects" aria-label="知画首页">
        <BrandLogo :size="46" />
        <span><strong>知画</strong></span>
      </RouterLink>
      <nav class="global-nav" aria-label="全局导航">
        <RouterLink v-for="item in nav" :key="item.path" :to="item.path">
          <component :is="item.icon" :size="18" stroke-width="2" />
          <span>{{ item.label }}</span>
        </RouterLink>
        <TaskCenter />
      </nav>
      <div class="appbar-drag" data-tauri-drag-region @mousedown="beginWindowDrag" @dblclick="toggleMaximizeWindow"></div>
      <RouterLink class="topbar-compute" to="/settings" title="打开算力设置"><ComputeStatus compact /></RouterLink>
      <button type="button" class="appbar-icon" aria-label="帮助" title="帮助" @click="helpOpen = true"><HelpCircle :size="18" /></button>
      <RouterLink class="appbar-icon" to="/settings" :class="{ active: isSettings }" aria-label="设置与算力" title="设置与算力"><Settings :size="18" /></RouterLink>
      <div class="window-actions">
        <button aria-label="最小化" @click="minimizeWindow"><Minus :size="16" /></button>
        <button aria-label="最大化" @click="toggleMaximizeWindow"><Square :size="13" /></button>
        <button aria-label="关闭" class="close" :disabled="closing" :title="closing ? '正在安全退出' : '关闭'" @click="closeWindow"><X :size="17" /></button>
      </div>
    </header>

    <main class="main-area" :class="{ 'with-project-bar': isProjectWorkspace }">
      <ProjectWorkspaceBar v-if="isProjectWorkspace" />
      <div class="route-view"><RouterView /></div>
    </main>

    <div v-if="helpOpen" class="help-backdrop" role="presentation" @click.self="helpOpen = false">
      <section class="help-dialog" role="dialog" aria-modal="true" aria-labelledby="help-title">
        <header>
          <div><BrandLogo :size="40" /><div><h2 id="help-title">从内容到成片</h2><p>把资料或想法，变成能直接修改的分镜视频。</p></div></div>
          <button type="button" aria-label="关闭帮助" @click="helpOpen = false"><X :size="20" /></button>
        </header>
        <ol>
          <li><b>选择开始方式</b><span>制作完整视频，或直接生成一个图片、视频素材。</span></li>
          <li><b>确认内容与分镜</b><span>核对知识点、事实和来源后再消耗 GPU 算力。</span></li>
          <li><b>生成候选视频</b><span>优云智算仅在需要生成时切换到 GPU，空闲后自动关机。</span></li>
          <li><b>采用结果并交付</b><span>可以导出单段素材，也可以继续完成旁白、字幕和 MP4 成片。</span></li>
        </ol>
        <footer><button type="button" class="btn primary" @click="helpOpen = false">知道了</button></footer>
      </section>
    </div>
  </div>
</template>
