<script setup lang="ts">
import { computed, ref } from "vue";
import { useRoute } from "vue-router";
import {
  FileText,
  Film,
  FolderKanban,
  HelpCircle,
  Images,
  Minus,
  Settings,
  Square,
  Upload,
  X,
} from "lucide-vue-next";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

const route = useRoute();
const appWindow = isTauri() ? getCurrentWindow() : null;
const isSettings = computed(() => route.path === "/settings");
const helpOpen = ref(false);

const minimizeWindow = () => appWindow?.minimize();
const toggleMaximizeWindow = () => appWindow?.toggleMaximize();
const closeWindow = () => appWindow?.close();

const nav = [
  { path: "/projects", label: "项目", icon: FolderKanban },
  { path: "/sources", label: "资料", icon: FileText },
  { path: "/storyboard", label: "分镜", icon: Film },
  { path: "/assets", label: "素材", icon: Images },
  { path: "/export", label: "导出", icon: Upload },
];
</script>

<template>
  <div class="app-frame">
    <div class="titlebar" data-tauri-drag-region>
      <div class="titlebar-name" data-tauri-drag-region>知画</div>
      <div class="window-actions">
        <button aria-label="最小化" @click="minimizeWindow"><Minus :size="16" /></button>
        <button aria-label="最大化" @click="toggleMaximizeWindow"><Square :size="13" /></button>
        <button aria-label="关闭" class="close" @click="closeWindow"><X :size="17" /></button>
      </div>
    </div>

    <aside class="sidebar">
      <div class="brand">
        <span class="brand-mark">知</span>
        <strong>知画</strong>
      </div>
      <nav class="main-nav">
        <RouterLink v-for="item in nav" :key="item.path" :to="item.path">
          <component :is="item.icon" :size="21" stroke-width="2" />
          <span>{{ item.label }}</span>
        </RouterLink>
      </nav>
      <div class="sidebar-footer">
        <button type="button" class="utility-link" aria-label="帮助" title="帮助" @click="helpOpen = true">
          <HelpCircle :size="20" /><span>帮助</span>
        </button>
        <RouterLink to="/settings" :class="{ active: isSettings }" aria-label="设置与算力" title="设置与算力">
          <Settings :size="20" /><span>设置</span>
        </RouterLink>
      </div>
    </aside>

    <main class="main-area">
      <RouterView />
    </main>

    <div v-if="helpOpen" class="help-backdrop" role="presentation" @click.self="helpOpen = false">
      <section class="help-dialog" role="dialog" aria-modal="true" aria-labelledby="help-title">
        <header>
          <div><span class="brand-mark">知</span><div><h2 id="help-title">开始使用知画</h2><p>按顺序完成资料、分镜、生成和导出。</p></div></div>
          <button type="button" aria-label="关闭帮助" @click="helpOpen = false"><X :size="20" /></button>
        </header>
        <ol>
          <li><b>创建项目</b><span>填写名称，选择目标受众与成片时长。</span></li>
          <li><b>导入并审核资料</b><span>核对知识点、事实和来源后再生成分镜。</span></li>
          <li><b>生成候选视频</b><span>优云智算仅在需要生成时切换到 GPU，空闲后自动关机。</span></li>
          <li><b>制作并导出成片</b><span>选择正式版本，制作 1080p 视频并导出 MP4。</span></li>
        </ol>
        <footer><button type="button" class="btn primary" @click="helpOpen = false">知道了</button></footer>
      </section>
    </div>
  </div>
</template>
