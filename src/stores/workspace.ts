import { computed, ref } from "vue";
import type { ZhihuaProject } from "../domain/projects";
import { projectRepository } from "../services/projectRepository";

const project = ref<ZhihuaProject>();
const loading = ref(false);

async function refresh() {
  loading.value = true;
  try {
    project.value = await projectRepository.active();
  } finally {
    loading.value = false;
  }
}

export function useWorkspaceStore() {
  const projectTitle = computed(() => project.value?.title ?? "尚未打开项目");
  const audience = computed(() => project.value?.audience || "未设置");
  const targetDurationSeconds = computed(() => project.value?.targetDurationSeconds ?? null);
  return { project, loading, projectTitle, audience, targetDurationSeconds, refresh };
}
