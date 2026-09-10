import { computed, ref, watch } from "vue";
import type {
  GenerationMode,
  SceneDraft,
  StoryboardProjectSettings,
} from "../domain/storyboard";
import { readLocal, writeLocal } from "../services/nativeBridge";
import {
  activeProjectId,
  storyboardRepository,
} from "../services/storyboardRepository";

const SETTINGS_KEY = "zhihua.storyboard.settings.v1";
const scenes = ref<SceneDraft[]>([]);
const settings = ref<StoryboardProjectSettings>(
  readLocal<StoryboardProjectSettings>(SETTINGS_KEY, {
    aspectRatio: "16:9",
    outputWidth: 1920,
    outputHeight: 1080,
    discardH3Audio: true,
  }),
);
const selectedSceneId = ref("");
const currentProjectId = ref<string>();
const loading = ref(false);
const loadError = ref("");
const persistTimers = new Map<string, number>();

watch(
  settings,
  () => writeLocal(SETTINGS_KEY, settings.value),
  { deep: true },
);

function replaceScene(saved: SceneDraft) {
  const index = scenes.value.findIndex((scene) => scene.id === saved.id);
  if (index >= 0) scenes.value[index] = saved;
}

function cloneScene(scene: SceneDraft): SceneDraft {
  return JSON.parse(JSON.stringify(scene)) as SceneDraft;
}

async function persistScene(scene: SceneDraft) {
  const timer = persistTimers.get(scene.id);
  if (timer !== undefined) window.clearTimeout(timer);
  persistTimers.delete(scene.id);
  try {
    replaceScene(await storyboardRepository.upsert(cloneScene(scene)));
    loadError.value = "";
  } catch (error) {
    loadError.value = error instanceof Error ? error.message : String(error);
  }
}

function schedulePersist(scene: SceneDraft) {
  const previous = persistTimers.get(scene.id);
  if (previous !== undefined) window.clearTimeout(previous);
  persistTimers.set(
    scene.id,
    window.setTimeout(() => void persistScene(scene), 300),
  );
}

async function load(force = false) {
  const projectId = activeProjectId();
  if (!projectId) {
    currentProjectId.value = undefined;
    scenes.value = [];
    selectedSceneId.value = "";
    loadError.value = "请先在项目页打开一个项目。";
    return;
  }
  if (!force && projectId === currentProjectId.value) return;
  loading.value = true;
  loadError.value = "";
  try {
    const loaded = await storyboardRepository.list(projectId);
    currentProjectId.value = projectId;
    scenes.value = loaded;
    selectedSceneId.value = loaded[0]?.id ?? "";
  } catch (error) {
    loadError.value = error instanceof Error ? error.message : String(error);
  } finally {
    loading.value = false;
  }
}

export function useStoryboardStore() {
  const selectedScene = computed(
    () =>
      scenes.value.find((scene) => scene.id === selectedSceneId.value) ??
      scenes.value[0],
  );

  if (activeProjectId() !== currentProjectId.value) void load();

  const select = (id: string) => {
    selectedSceneId.value = id;
  };
  const update = (id: string, patch: Partial<SceneDraft>) => {
    const scene = scenes.value.find((item) => item.id === id);
    if (!scene) return;
    Object.assign(scene, patch, { updatedAt: new Date().toISOString() });
    schedulePersist(scene);
  };
  const save = async (id: string, patch: Partial<SceneDraft>) => {
    const scene = scenes.value.find((item) => item.id === id);
    if (!scene) return;
    Object.assign(scene, patch, { updatedAt: new Date().toISOString() });
    await persistScene(scene);
  };
  const updateSelected = (patch: Partial<SceneDraft>) => {
    const scene = selectedScene.value;
    if (scene) update(scene.id, patch);
  };
  const setMode = (mode: GenerationMode) =>
    updateSelected({ generationMode: mode });
  const add = async () => {
    const projectId = currentProjectId.value ?? activeProjectId();
    if (!projectId) {
      loadError.value = "请先打开项目，再新增分镜。";
      return;
    }
    const order = scenes.value.length;
    const timestamp = new Date().toISOString();
    const scene: SceneDraft = {
      id: crypto.randomUUID(),
      projectId,
      order,
      title: `新分镜 ${order + 1}`,
      purpose: "",
      sourceRefs: [],
      narration: "",
      onScreenText: [],
      visualPlan: "",
      generationMode: "t2v",
      targetDurationMs: 5000,
      assetIds: [],
      status: "draft",
      quality: "fast",
      updatedAt: timestamp,
    };
    scenes.value.push(scene);
    selectedSceneId.value = scene.id;
    await persistScene(scene);
  };
  const duplicateSelected = async () => {
    const source = selectedScene.value;
    if (!source) return;
    const index = scenes.value.indexOf(source) + 1;
    const clone: SceneDraft = {
      ...cloneScene(source),
      id: crypto.randomUUID(),
      order: index,
      title: `${source.title}（副本）`,
      selectedVersionId: undefined,
      lastJobId: undefined,
      pendingRequestId: undefined,
      generationStage: undefined,
      status: "draft",
      updatedAt: new Date().toISOString(),
    };
    scenes.value.splice(index, 0, clone);
    scenes.value.forEach((scene, order) => {
      scene.order = order;
    });
    selectedSceneId.value = clone.id;
    await persistScene(clone);
    await storyboardRepository.reorder(
      source.projectId,
      scenes.value.map((scene) => scene.id),
    );
  };
  const removeSelected = async () => {
    if (scenes.value.length <= 1) return false;
    const index = scenes.value.findIndex(
      (scene) => scene.id === selectedSceneId.value,
    );
    if (index < 0) return false;
    const [removed] = scenes.value.splice(index, 1);
    const timer = persistTimers.get(removed.id);
    if (timer !== undefined) window.clearTimeout(timer);
    persistTimers.delete(removed.id);
    scenes.value.forEach((scene, order) => {
      scene.order = order;
    });
    selectedSceneId.value =
      scenes.value[Math.min(index, scenes.value.length - 1)].id;
    await storyboardRepository.remove(removed.projectId, removed.id);
    return true;
  };
  const moveSelected = async (offset: -1 | 1) => {
    const index = scenes.value.findIndex(
      (scene) => scene.id === selectedSceneId.value,
    );
    const target = index + offset;
    if (index < 0 || target < 0 || target >= scenes.value.length) return;
    [scenes.value[index], scenes.value[target]] = [
      scenes.value[target],
      scenes.value[index],
    ];
    scenes.value.forEach((scene, order) => {
      scene.order = order;
    });
    const projectId = currentProjectId.value;
    if (projectId) {
      await storyboardRepository.reorder(
        projectId,
        scenes.value.map((scene) => scene.id),
      );
    }
  };

  return {
    scenes,
    settings,
    selectedSceneId,
    selectedScene,
    currentProjectId,
    loading,
    loadError,
    load,
    select,
    update,
    save,
    updateSelected,
    setMode,
    add,
    duplicateSelected,
    removeSelected,
    moveSelected,
  };
}
