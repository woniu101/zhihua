<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import { useRoute } from "vue-router";
import { ArrowLeft, BookOpenText, Film, Images, PlaySquare } from "lucide-vue-next";
import { useWorkspaceStore } from "../stores/workspace";
import { writeLocal } from "../services/nativeBridge";

const route = useRoute();
const workspace = useWorkspaceStore();

const stages = [
  { path: "/sources", label: "内容", hint: "资料与大纲", icon: BookOpenText },
  { path: "/storyboard", label: "分镜", hint: "镜头与生成", icon: Film },
  { path: "/assets", label: "素材", hint: "参考与结果", icon: Images },
  { path: "/export", label: "成片", hint: "预览与导出", icon: PlaySquare },
];

const stateHint = computed(() => {
  const project = workspace.project.value;
  if (!project) return "";
  if (project.state === "已完成") return "成片已导出";
  if (project.state === "待导出") return "镜头已就绪，建议检查成片";
  if (project.state === "生成中") return "生成任务正在后台运行";
  if (project.shotCount) return `${project.shotCount} 个镜头，继续完善分镜`;
  return "从内容或分镜开始";
});

async function refreshWorkspace() {
  await workspace.refresh().catch(() => undefined);
}

onMounted(refreshWorkspace);
watch(() => route.path, async (path) => {
  await refreshWorkspace();
  const projectId = workspace.project.value?.id;
  if (projectId && stages.some((stage) => stage.path === path)) {
    writeLocal(`zhihua.lastProjectRoute.${projectId}`, path);
  }
});
</script>

<template>
  <section class="project-workspace-bar" aria-label="当前项目工作区">
    <RouterLink class="back-projects" to="/projects" title="返回全部项目">
      <ArrowLeft :size="17" />
      <span>全部项目</span>
    </RouterLink>
    <div class="active-project-name">
      <small>当前项目</small>
      <strong>{{ workspace.projectTitle.value }}</strong>
    </div>
    <nav class="project-stage-nav" aria-label="项目制作步骤">
      <RouterLink v-for="stage in stages" :key="stage.path" :to="stage.path">
        <component :is="stage.icon" :size="17" />
        <span><b>{{ stage.label }}</b><small>{{ stage.hint }}</small></span>
      </RouterLink>
    </nav>
    <div class="project-progress-note">
      <i></i>
      <span>{{ stateHint }}</span>
    </div>
  </section>
</template>
