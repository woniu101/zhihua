import { computed, ref } from "vue";
import type { KnowledgePoint, SourceDocument, SourceKind } from "../domain/sources";
import { sourceRepository } from "../services/sourceRepository";

const items = ref<SourceDocument[]>([]);
const points = ref<KnowledgePoint[]>([]);
const selectedId = ref<string>();
const filter = ref<"全部" | SourceKind>("全部");
const loading = ref(false);

export function useSourceStore() {
  const visibleSources = computed(() => filter.value === "全部" ? items.value : items.value.filter((item) => item.kind === filter.value));
  const selected = computed(() => items.value.find((item) => item.id === selectedId.value) ?? visibleSources.value[0]);
  const enabledReadyCount = computed(() => items.value.filter((item) => item.enabled && item.status === "ready").length);
  const unresolvedCount = computed(() => points.value.filter((item) => item.needsConfirmation && !item.confirmed).length);

  const persist = () => sourceRepository.save(items.value);
  const persistPoints = () => sourceRepository.savePoints(points.value);

  const load = async () => {
    loading.value = true;
    try {
      [items.value, points.value] = await Promise.all([sourceRepository.list(), sourceRepository.listKnowledgePoints()]);
      selectedId.value ||= items.value[0]?.id;
    } finally { loading.value = false; }
  };

  const importFiles = async (files: File[]) => {
    for (const file of files) {
      const source = await sourceRepository.fromFile(file);
      items.value.unshift(source);
      selectedId.value = source.id;
      persist();
    }
  };

  const importNative = async () => {
    const sources = await sourceRepository.pickNative();
    if (sources?.length) {
      items.value.unshift(...sources);
      selectedId.value = sources[0].id;
      persist();
      return true;
    }
    return false;
  };

  const pasteText = async (text: string, name: string) => {
    const source = await sourceRepository.fromText(text, name);
    items.value.unshift(source);
    selectedId.value = source.id;
    persist();
  };

  const toggle = async (source: SourceDocument) => {
    source.enabled = !source.enabled;
    persist();
    await sourceRepository.setEnabled(source.id, source.enabled);
  };

  const remove = async (id: string) => {
    await sourceRepository.remove(id);
    items.value = items.value.filter((item) => item.id !== id);
    points.value = points.value.map((point) => ({ ...point, sourceRefs: point.sourceRefs.filter((ref) => ref.sourceId !== id) }));
    if (selectedId.value === id) selectedId.value = items.value[0]?.id;
    persist();
    persistPoints();
  };

  const updatePoint = (id: string, patch: Partial<KnowledgePoint>) => {
    points.value = points.value.map((point) => point.id === id ? { ...point, ...patch } : point);
    persistPoints();
  };

  const addPoint = () => {
    const source = selected.value;
    points.value.push({
      id: crypto.randomUUID(), title: "新的知识点", detail: "点击编辑知识点说明。",
      needsConfirmation: true, confirmed: false,
      sourceRefs: source ? [{ sourceId: source.id, sourceName: source.name, location: "手动添加" }] : [],
    });
    persistPoints();
  };

  const extractKnowledge = async () => {
    const sourceIds = items.value.filter((item) => item.enabled && item.status === "ready").map((item) => item.id);
    const extracted = await sourceRepository.extractKnowledge(sourceIds);
    if (!extracted) return false;
    points.value = extracted;
    persistPoints();
    return true;
  };

  return { items, points, selectedId, selected, filter, loading, visibleSources, enabledReadyCount, unresolvedCount, load, importFiles, importNative, pasteText, toggle, remove, updatePoint, addPoint, extractKnowledge };
}
