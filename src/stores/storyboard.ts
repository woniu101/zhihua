import { computed, ref, watch } from "vue";
import type {
  GenerationMode,
  SceneDraft,
  StoryboardProjectSettings,
  VisualIntent,
} from "../domain/storyboard";
import { readLocal, writeLocal } from "../services/nativeBridge";
import {
  activeProjectId,
  storyboardRepository,
} from "../services/storyboardRepository";

function settingsKey(projectId: string): string {
  return `zhihua.storyboard.settings.${projectId}.v3`;
}

function defaultSettings(): StoryboardProjectSettings {
  return {
    aspectRatio: "16:9",
    h3AudioPolicy: "smart",
  };
}

const scenes = ref<SceneDraft[]>([]);
const settings = ref<StoryboardProjectSettings>(defaultSettings());
const selectedSceneId = ref("");
const currentProjectId = ref<string>();
const loading = ref(false);
const loadError = ref("");
const persistTimers = new Map<string, number>();
const persistChains = new Map<string, Promise<boolean>>();
let editingStructure = false;

function blankVisualIntent(): VisualIntent {
  return { subject: "", action: "", scene: "", composition: "", camera: "", lighting: "", timeline: "", negative: "" };
}

function newScene(projectId: string, order: number): SceneDraft {
  return {
    id: crypto.randomUUID(), projectId, order, title: `新分镜 ${order + 1}`, purpose: "",
    sourceRefs: [], narration: "", narrationMode: "tts", ambientSound: "与画面动作同步的自然环境声",
    onScreenText: [], visualPlan: "", visualIntent: blankVisualIntent(), promptMode: "quick", audioIntent: "environment", locked: false,
    generationMode: "t2v", targetDurationMs: 5000, assetIds: [], status: "draft", quality: "fast",
    updatedAt: new Date().toISOString(),
  };
}

function normalizeScene(scene: SceneDraft): SceneDraft {
  return {
    ...scene,
    narrationMode: scene.narrationMode ?? "tts",
    ambientSound: scene.ambientSound ?? "与画面动作同步的自然环境声",
    visualIntent: { ...blankVisualIntent(), ...(scene.visualIntent ?? {}) },
    promptMode: scene.promptMode ?? "quick",
    audioIntent: scene.audioIntent ?? "environment",
    locked: scene.locked ?? false,
  };
}

watch(
  settings,
  () => {
    if (currentProjectId.value) {
      writeLocal(settingsKey(currentProjectId.value), settings.value);
    }
  },
  { deep: true },
);

function replaceScene(saved: SceneDraft) {
  const index = scenes.value.findIndex((scene) => scene.id === saved.id);
  if (index >= 0) scenes.value[index] = saved;
}

function cloneScene(scene: SceneDraft): SceneDraft {
  return JSON.parse(JSON.stringify(scene)) as SceneDraft;
}

function composeVisualIntent(intent: VisualIntent): string {
  return [
    intent.subject && `主体：${intent.subject}`,
    intent.action && `动作：${intent.action}`,
    intent.scene && `场景：${intent.scene}`,
    intent.composition && `构图：${intent.composition}`,
    intent.camera && `镜头：${intent.camera}`,
    intent.lighting && `光线与风格：${intent.lighting}`,
    intent.timeline && `时间线：${intent.timeline}`,
    intent.negative && `避免：${intent.negative}`,
  ].filter(Boolean).join("；");
}

function effectiveVisualPlan(scene: SceneDraft): string {
  return scene.promptMode === "advanced"
    ? composeVisualIntent(scene.visualIntent).trim()
    : scene.visualPlan.trim();
}

async function persistScene(scene: SceneDraft): Promise<boolean> {
  const timer = persistTimers.get(scene.id);
  if (timer !== undefined) window.clearTimeout(timer);
  persistTimers.delete(scene.id);
  const snapshot = cloneScene(scene);
  const previous = persistChains.get(scene.id) ?? Promise.resolve(true);
  const operation = previous.then(async () => {
    try {
      const saved = normalizeScene(await storyboardRepository.upsert(snapshot));
      const current = scenes.value.find((item) => item.id === saved.id);
      if (current?.updatedAt === snapshot.updatedAt) replaceScene(saved);
      loadError.value = "";
      return true;
    } catch (error) {
      loadError.value = error instanceof Error ? error.message : String(error);
      return false;
    }
  });
  persistChains.set(scene.id, operation);
  const result = await operation;
  if (persistChains.get(scene.id) === operation) persistChains.delete(scene.id);
  return result;
}

function schedulePersist(scene: SceneDraft) {
  const previous = persistTimers.get(scene.id);
  if (previous !== undefined) window.clearTimeout(previous);
  persistTimers.set(
    scene.id,
    window.setTimeout(() => void persistScene(scene), 300),
  );
}

async function prepareStructuralEdit(): Promise<SceneDraft[] | undefined> {
  if (editingStructure) return undefined;
  editingStructure = true;
  const saved = await Promise.all(scenes.value.map((scene) => persistScene(scene)));
  if (saved.some((result) => !result)) {
    editingStructure = false;
    return undefined;
  }
  return scenes.value.map(cloneScene);
}

async function commitStructuralEdit(
  expected: SceneDraft[],
  next: SceneDraft[],
  nextSelection: string,
): Promise<boolean> {
  const projectId = currentProjectId.value ?? activeProjectId();
  if (!projectId) {
    editingStructure = false;
    return false;
  }
  try {
    scenes.value = (await storyboardRepository.applyEdit(projectId, expected, next)).map(normalizeScene);
    selectedSceneId.value = scenes.value.some((scene) => scene.id === nextSelection)
      ? nextSelection
      : scenes.value[0]?.id ?? "";
    loadError.value = "";
    return true;
  } catch (error) {
    loadError.value = error instanceof Error ? error.message : String(error);
    await load(true);
    return false;
  } finally {
    editingStructure = false;
  }
}

function splitText(text: string): [string, string] {
  const value = text.trim();
  if (!value) return ["", ""];
  const sentences = value.split(/(?<=[。！？!?])/).filter(Boolean);
  if (sentences.length > 1) {
    const midpoint = Math.ceil(sentences.length / 2);
    return [sentences.slice(0, midpoint).join(""), sentences.slice(midpoint).join("")];
  }
  const characters = Array.from(value);
  const midpoint = Math.max(1, Math.ceil(characters.length / 2));
  return [characters.slice(0, midpoint).join(""), characters.slice(midpoint).join("")];
}

async function load(force = false) {
  const projectId = activeProjectId();
  if (!projectId) {
    currentProjectId.value = undefined;
    scenes.value = [];
    selectedSceneId.value = "";
    loadError.value = "请先在项目页打开一个项目。";
    settings.value = defaultSettings();
    return;
  }
  if (!force && projectId === currentProjectId.value) return;
  loading.value = true;
  loadError.value = "";
  const previousSelection = selectedSceneId.value;
  try {
    const loaded = await storyboardRepository.list(projectId);
    currentProjectId.value = projectId;
    settings.value = readLocal<StoryboardProjectSettings>(settingsKey(projectId), defaultSettings());
    scenes.value = loaded.map(normalizeScene);
    selectedSceneId.value = loaded.some((scene) => scene.id === previousSelection)
      ? previousSelection
      : loaded[0]?.id ?? "";
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
    if (scene.locked && patch.locked === undefined) return;
    Object.assign(scene, patch, { updatedAt: new Date().toISOString() });
    schedulePersist(scene);
  };
  const save = async (id: string, patch: Partial<SceneDraft>) => {
    const scene = scenes.value.find((item) => item.id === id);
    if (!scene) return false;
    if (scene.locked && patch.locked === undefined) return false;
    Object.assign(scene, patch, { updatedAt: new Date().toISOString() });
    return persistScene(scene);
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
    const expected = await prepareStructuralEdit();
    if (!expected) return;
    const scene = newScene(projectId, expected.length);
    await commitStructuralEdit(expected, [...expected, scene], scene.id);
  };
  const duplicateSelected = async () => {
    const source = selectedScene.value;
    if (!source) return;
    const expected = await prepareStructuralEdit();
    if (!expected) return;
    const persistedSource = expected.find((scene) => scene.id === source.id);
    if (!persistedSource) {
      editingStructure = false;
      return;
    }
    const index = expected.findIndex((scene) => scene.id === source.id) + 1;
    const clone: SceneDraft = {
      ...cloneScene(persistedSource),
      id: crypto.randomUUID(),
      order: index,
      title: `${persistedSource.title}（副本）`,
      selectedVersionId: undefined,
      lastJobId: undefined,
      lastUpscaleJobId: undefined,
      pendingRequestId: undefined,
      generationStage: undefined,
      locked: false,
      status: "draft",
      updatedAt: new Date().toISOString(),
    };
    const next = expected.map(cloneScene);
    next.splice(index, 0, clone);
    await commitStructuralEdit(expected, next, clone.id);
  };
  const removeSelected = async () => {
    if (scenes.value.length <= 1) return false;
    const source = selectedScene.value;
    if (!source || source.locked || source.status === "generating" || source.pendingRequestId) return false;
    const expected = await prepareStructuralEdit();
    if (!expected) return false;
    const index = expected.findIndex((scene) => scene.id === source.id);
    if (index < 0) {
      editingStructure = false;
      return false;
    }
    const next = expected.filter((scene) => scene.id !== source.id);
    const nextSelection = next[Math.min(index, next.length - 1)]?.id ?? "";
    return commitStructuralEdit(expected, next, nextSelection);
  };
  const moveSelected = async (offset: -1 | 1) => {
    const selectedId = selectedSceneId.value;
    const index = scenes.value.findIndex((scene) => scene.id === selectedId);
    const target = index + offset;
    if (index < 0 || target < 0 || target >= scenes.value.length) return;
    const expected = await prepareStructuralEdit();
    if (!expected) return;
    const persistedIndex = expected.findIndex((scene) => scene.id === selectedId);
    const persistedTarget = persistedIndex + offset;
    if (persistedIndex < 0 || persistedTarget < 0 || persistedTarget >= expected.length) {
      editingStructure = false;
      return;
    }
    const next = expected.map(cloneScene);
    [next[persistedIndex], next[persistedTarget]] = [next[persistedTarget], next[persistedIndex]];
    await commitStructuralEdit(expected, next, selectedId);
  };

  const insertAt = async (index: number) => {
    const projectId = currentProjectId.value ?? activeProjectId();
    if (!projectId) return;
    const expected = await prepareStructuralEdit();
    if (!expected) return;
    const safeIndex = Math.max(0, Math.min(index, expected.length));
    const scene = newScene(projectId, safeIndex);
    const next = expected.map(cloneScene);
    next.splice(safeIndex, 0, scene);
    await commitStructuralEdit(expected, next, scene.id);
  };

  const insertBeforeSelected = async () => {
    const index = scenes.value.findIndex((scene) => scene.id === selectedSceneId.value);
    await insertAt(index < 0 ? scenes.value.length : index);
  };

  const insertAfterSelected = async () => {
    const index = scenes.value.findIndex((scene) => scene.id === selectedSceneId.value);
    await insertAt(index < 0 ? scenes.value.length : index + 1);
  };

  const toggleSelectedLock = async () => {
    const scene = selectedScene.value;
    if (scene && scene.status !== "generating" && !scene.pendingRequestId) {
      await save(scene.id, { locked: !scene.locked });
    }
  };

  const splitSelected = async () => {
    const source = selectedScene.value;
    if (!source || source.locked || source.status === "generating" || source.pendingRequestId || source.targetDurationMs < 8000) return false;
    const expected = await prepareStructuralEdit();
    if (!expected) return false;
    const index = expected.findIndex((scene) => scene.id === source.id);
    const persistedSource = expected[index];
    if (!persistedSource) {
      editingStructure = false;
      return false;
    }
    const totalSeconds = Math.round(persistedSource.targetDurationMs / 1000);
    const firstDuration = Math.floor(totalSeconds / 2) * 1000;
    const secondDuration = (totalSeconds - Math.floor(totalSeconds / 2)) * 1000;
    const [firstNarration, secondNarration] = splitText(persistedSource.narration);
    const baseTitle = persistedSource.title.replace(/（前半）$|（后半）$/, "");
    const first = cloneScene(persistedSource);
    const second: SceneDraft = {
      ...cloneScene(persistedSource), id: crypto.randomUUID(), order: index + 1,
      title: `${baseTitle}（后半）`, narration: secondNarration,
      visualPlan: persistedSource.visualPlan ? `${persistedSource.visualPlan}；只表现动作的后半阶段与结果` : "",
      visualIntent: {
        ...persistedSource.visualIntent,
        timeline: [persistedSource.visualIntent.timeline, "只表现后半阶段与结果"].filter(Boolean).join("；"),
      },
      targetDurationMs: secondDuration, selectedVersionId: undefined, lastJobId: undefined,
      lastUpscaleJobId: undefined, pendingRequestId: undefined, generationStage: undefined,
      status: "draft", locked: false, updatedAt: new Date().toISOString(),
    };
    Object.assign(first, {
      title: `${baseTitle}（前半）`, narration: firstNarration,
      visualPlan: persistedSource.visualPlan ? `${persistedSource.visualPlan}；只表现动作的前半阶段` : "",
      visualIntent: {
        ...persistedSource.visualIntent,
        timeline: [persistedSource.visualIntent.timeline, "只表现前半阶段"].filter(Boolean).join("；"),
      },
      targetDurationMs: firstDuration, selectedVersionId: undefined, lastJobId: undefined,
      lastUpscaleJobId: undefined, pendingRequestId: undefined, generationStage: undefined,
      status: "draft", updatedAt: new Date().toISOString(),
    });
    const next = expected.map(cloneScene);
    next.splice(index, 1, first, second);
    return commitStructuralEdit(expected, next, second.id);
  };

  const mergeSelectedWithNext = async () => {
    const source = selectedScene.value;
    if (!source || source.locked || source.status === "generating" || source.pendingRequestId) return false;
    const expected = await prepareStructuralEdit();
    if (!expected) return false;
    const index = expected.findIndex((scene) => scene.id === source.id);
    const first = expected[index];
    const second = expected[index + 1];
    if (!first || !second || second.locked || second.status === "generating" || second.pendingRequestId || first.targetDurationMs + second.targetDurationMs > 15000) {
      editingStructure = false;
      return false;
    }
    if (first.generationMode !== second.generationMode || first.audioIntent !== second.audioIntent) {
      loadError.value = "合并前请先统一两个分镜的生成方式和原声音频意图。";
      editingStructure = false;
      return false;
    }
    const duration = first.targetDurationMs + second.targetDurationMs;
    const merged = cloneScene(first);
    Object.assign(merged, {
      title: first.title.replace(/（前半）$/, ""),
      purpose: [first.purpose, second.purpose].filter(Boolean).join("；"),
      narration: [first.narration, second.narration].filter(Boolean).join(""),
      onScreenText: [...first.onScreenText, ...second.onScreenText].slice(0, 2),
      visualPlan: [effectiveVisualPlan(first), effectiveVisualPlan(second)].filter(Boolean).join("；随后，"),
      visualIntent: blankVisualIntent(),
      promptMode: "quick",
      ambientSound: [first.ambientSound, second.ambientSound].filter(Boolean).join("；"),
      sourceRefs: [...first.sourceRefs, ...second.sourceRefs.filter((ref) => !first.sourceRefs.some((item) => JSON.stringify(item) === JSON.stringify(ref)))],
      assetIds: [...new Set([...first.assetIds, ...second.assetIds])],
      targetDurationMs: duration, selectedVersionId: undefined, lastJobId: undefined,
      lastUpscaleJobId: undefined, pendingRequestId: undefined, generationStage: undefined,
      status: "draft", updatedAt: new Date().toISOString(),
    });
    const next = expected.map(cloneScene);
    next.splice(index, 2, merged);
    return commitStructuralEdit(expected, next, merged.id);
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
    insertBeforeSelected,
    insertAfterSelected,
    splitSelected,
    mergeSelectedWithNext,
    toggleSelectedLock,
  };
}
