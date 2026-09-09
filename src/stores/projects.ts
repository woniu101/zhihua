import { computed, ref } from "vue";
import type { NewProjectInput, ProjectState, ZhihuaProject } from "../domain/projects";
import { projectRepository } from "../services/projectRepository";

const items = ref<ZhihuaProject[]>([]);
const loading = ref(false);
const query = ref("");
const status = ref<"全部" | ProjectState>("全部");
const sortDescending = ref(true);

export function useProjectStore() {
  const visibleProjects = computed(() => {
    const needle = query.value.trim().toLocaleLowerCase();
    return items.value
      .filter((project) => status.value === "全部" || project.state === status.value)
      .filter((project) => !needle || `${project.title} ${project.description} ${project.audience}`.toLocaleLowerCase().includes(needle))
      .sort((a, b) => sortDescending.value
        ? b.updatedAt.localeCompare(a.updatedAt)
        : a.updatedAt.localeCompare(b.updatedAt));
  });

  const load = async () => {
    loading.value = true;
    try { items.value = await projectRepository.list(); }
    finally { loading.value = false; }
  };

  const create = async (input: NewProjectInput) => {
    const project = await projectRepository.create(input);
    items.value = [project, ...items.value.filter((item) => item.id !== project.id)];
    return project;
  };

  const rename = async (id: string, title: string) => {
    await projectRepository.rename(id, title);
    items.value = items.value.map((item) => item.id === id ? { ...item, title, updatedAt: new Date().toISOString() } : item);
  };

  const duplicate = async (id: string) => {
    const project = await projectRepository.duplicate(id);
    if (project) items.value = [project, ...items.value];
  };

  const remove = async (id: string) => {
    await projectRepository.remove(id);
    items.value = items.value.filter((item) => item.id !== id);
  };

  const open = async (id: string) => {
    await projectRepository.markOpened(id);
    const timestamp = new Date().toISOString();
    items.value = items.value.map((item) => item.id === id ? { ...item, lastOpenedAt: timestamp, updatedAt: timestamp } : item);
  };

  return { items, loading, query, status, sortDescending, visibleProjects, load, create, rename, duplicate, remove, open };
}
