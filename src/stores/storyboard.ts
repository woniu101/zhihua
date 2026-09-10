import { computed, ref, watch } from "vue";
import type { GenerationMode, SceneDraft, StoryboardProjectSettings } from "../domain/storyboard";
import { readLocal, writeLocal } from "../services/nativeBridge";

const STORAGE_KEY = "zhihua.storyboard.demo.v1";

const initialScenes: SceneDraft[] = [
  ["乌云正在聚集", "展现乌云中电荷逐渐聚集", "带电的云层中，正负电荷开始分离。", "乌云翻涌，云层上下电荷逐渐聚集，远处山谷保持稳定。"],
  ["电荷开始分离", "解释电场持续增强", "电荷分离导致云层之间的电场不断增强。", "云层上方蓝色负电荷、下方橙色正电荷分布更加明显。"],
  ["闪电划破天空", "解释空气被击穿", "当电场足够强，空气就会被击穿。", "强烈闪电从云层贯穿到地面，画面主体保持在安全区。"],
  ["雷声随后传来", "解释雷声形成", "闪电让空气迅速受热膨胀，冲击波形成雷声。", "闪电余光照亮天空，空气波纹向外扩散。"],
  ["雷雨天如何避险", "给出安全建议", "雷雨天气应远离高处、孤立大树和金属设施。", "用简洁图示表现室内避险和远离孤立高物。"],
].map((item, index) => ({
  id: `scene-${index + 1}`,
  projectId: "demo-lightning",
  order: index,
  title: item[0],
  purpose: item[1],
  sourceRefs: [{ sourceId: "source-lightning", page: index + 2 }],
  narration: item[2],
  onScreenText: [item[0]],
  visualPlan: item[3],
  generationMode: index === 0 ? "t2v" : "i2v",
  targetDurationMs: index === 0 ? 5000 : 10000,
  assetIds: [],
  selectedVersionId: index < 2 ? `version-${index + 1}` : undefined,
  status: index === 0 ? "approved" : index === 1 ? "generated" : "draft",
  quality: "fast",
  updatedAt: new Date().toISOString(),
})) as SceneDraft[];

const saved = readLocal<{ scenes: SceneDraft[]; settings: StoryboardProjectSettings } | null>(STORAGE_KEY, null);
const scenes = ref<SceneDraft[]>(saved?.scenes?.length ? saved.scenes : initialScenes);
const settings = ref<StoryboardProjectSettings>(saved?.settings ?? {
  aspectRatio: "16:9",
  outputWidth: 1920,
  outputHeight: 1080,
  discardH3Audio: true,
});
const selectedSceneId = ref(scenes.value[0]?.id ?? "");

watch([scenes, settings], () => writeLocal(STORAGE_KEY, { scenes: scenes.value, settings: settings.value }), { deep: true });

export function useStoryboardStore() {
  const selectedScene = computed(() => scenes.value.find((scene) => scene.id === selectedSceneId.value) ?? scenes.value[0]);

  const update = (id: string, patch: Partial<SceneDraft>) => {
    const scene = scenes.value.find((item) => item.id === id);
    if (!scene) return;
    Object.assign(scene, patch, { updatedAt: new Date().toISOString() });
  };
  const select = (id: string) => { selectedSceneId.value = id; };
  const updateSelected = (patch: Partial<SceneDraft>) => {
    const scene = selectedScene.value;
    if (!scene) return;
    update(scene.id, patch);
  };
  const setMode = (mode: GenerationMode) => updateSelected({ generationMode: mode });
  const add = () => {
    const order = scenes.value.length;
    const scene: SceneDraft = {
      id: crypto.randomUUID(), projectId: "demo-lightning", order,
      title: `新分镜 ${order + 1}`, purpose: "", sourceRefs: [], narration: "", onScreenText: [], visualPlan: "",
      generationMode: "t2v", targetDurationMs: 5000, assetIds: [], status: "draft", quality: "fast", updatedAt: new Date().toISOString(),
    };
    scenes.value.push(scene);
    selectedSceneId.value = scene.id;
  };
  const duplicateSelected = () => {
    const source = selectedScene.value;
    if (!source) return;
    const index = scenes.value.indexOf(source) + 1;
    const clone: SceneDraft = { ...structuredClone(source), id: crypto.randomUUID(), title: `${source.title}（副本）`, selectedVersionId: undefined, status: "draft", updatedAt: new Date().toISOString() };
    scenes.value.splice(index, 0, clone);
    scenes.value.forEach((scene, order) => { scene.order = order; });
    selectedSceneId.value = clone.id;
  };
  const removeSelected = () => {
    if (scenes.value.length <= 1) return false;
    const index = scenes.value.findIndex((scene) => scene.id === selectedSceneId.value);
    if (index < 0) return false;
    scenes.value.splice(index, 1);
    scenes.value.forEach((scene, order) => { scene.order = order; });
    selectedSceneId.value = scenes.value[Math.min(index, scenes.value.length - 1)].id;
    return true;
  };
  const moveSelected = (offset: -1 | 1) => {
    const index = scenes.value.findIndex((scene) => scene.id === selectedSceneId.value);
    const target = index + offset;
    if (index < 0 || target < 0 || target >= scenes.value.length) return;
    [scenes.value[index], scenes.value[target]] = [scenes.value[target], scenes.value[index]];
    scenes.value.forEach((scene, order) => { scene.order = order; });
  };

  return { scenes, settings, selectedSceneId, selectedScene, select, update, updateSelected, setMode, add, duplicateSelected, removeSelected, moveSelected };
}
