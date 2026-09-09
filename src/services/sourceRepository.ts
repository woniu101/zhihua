import type { KnowledgePoint, SourceDocument, SourceKind } from "../domain/sources";
import { open } from "@tauri-apps/plugin-dialog";
import { invokeNative, invokeOptional, isNativeRuntime, readLocal, writeLocal } from "./nativeBridge";

const SOURCE_KEY = "zhihua.sources.v1";
const POINT_KEY = "zhihua.knowledgePoints.v1";

const timestamp = new Date().toISOString();
const demoSources: SourceDocument[] = [
  { id: "source-pdf", name: "为什么会打雷？.pdf", kind: "PDF", size: 3_200_000, pageCount: 12, enabled: true, status: "ready", progress: 100, statusMessage: "解析完成", createdAt: timestamp, extractedText: "2. 为什么会打雷？\n\n打雷是云层中发生剧烈放电时产生的声音。积雨云中存在大量的正、负电荷，当电荷积累到一定程度，空气的绝缘性被击穿，形成强大的电流通道，也就是闪电。\n\n闪电发生时，电流会使周围空气在极短时间内迅速加热，温度可达 3 万摄氏度以上。受热的空气迅速膨胀，形成强烈的冲击波，这就是我们听到的雷声。\n\n由于光的传播速度比声音快得多，所以我们总是先看到闪电，再听到雷声。" },
  { id: "source-pptx", name: "天气与气候.pptx", kind: "PPTX", size: 18_600_000, pageCount: 26, enabled: true, status: "parsing", progress: 60, statusMessage: "解析中 60%", createdAt: timestamp, extractedText: "正在等待桌面解析器返回提取内容。" },
  { id: "source-docx", name: "雷电科学原理.docx", kind: "DOCX", size: 1_100_000, pageCount: 8, enabled: true, status: "error", progress: 0, statusMessage: "解析失败 · 可重试", createdAt: timestamp, extractedText: "该演示文件解析失败。真实 DOCX 解析需由桌面端解析 command 提供。" },
  { id: "source-txt", name: "常见气象问题.txt", kind: "TXT", size: 256_000, enabled: true, status: "ready", progress: 100, statusMessage: "解析完成", createdAt: timestamp, extractedText: "雷电天气常见问题：为什么先看到闪电，后听到雷声？" },
  { id: "source-pdf-2", name: "闪电的类型与防护.pdf", kind: "PDF", size: 4_800_000, pageCount: 15, enabled: true, status: "ready", progress: 100, statusMessage: "解析完成", createdAt: timestamp, extractedText: "雷雨天气应远离高处、孤立大树和金属设施，并尽快进入安全建筑。" },
];

const demoPoints: KnowledgePoint[] = [
  ["云层中正负电荷的形成与积累", "积雨云中存在大量正负电荷，积累到一定程度就可能发生放电。", "第 2 页", false],
  ["空气被击穿形成闪电", "强电场击穿空气，形成短时间的强大电流通道。", "第 3 页", false],
  ["闪电使空气迅速加热并产生冲击波", "温度数据需要与其他来源交叉核对。", "第 3 页", true],
  ["先看到闪电后听到雷声的原因", "光速远高于声速，因此视觉信号先到达。", "第 3 页", false],
  ["根据时间差估算距离", "通过闪电与雷声间隔可粗略估计距离。", "第 4 页", false],
].map(([title, detail, location, needsConfirmation], index) => ({
  id: `point-${index + 1}`,
  title: title as string,
  detail: detail as string,
  needsConfirmation: needsConfirmation as boolean,
  confirmed: !(needsConfirmation as boolean),
  sourceRefs: [{ sourceId: "source-pdf", sourceName: "为什么会打雷？.pdf", location: location as string }],
}));

function kindFromFile(file: File): SourceKind | undefined {
  const extension = file.name.split(".").pop()?.toLocaleLowerCase();
  if (extension === "pdf") return "PDF";
  if (extension === "pptx") return "PPTX";
  if (extension === "docx") return "DOCX";
  if (extension === "txt") return "TXT";
  if (["png", "jpg", "jpeg", "webp", "gif", "bmp"].includes(extension ?? "")) return "IMAGE";
  return undefined;
}

type NativeSourceType = "txt" | "pdf" | "docx" | "pptx" | "image" | "pasted_text";
type NativeParseStatus = "ready" | "no_text" | "failed";
interface NativeSource {
  id: string;
  projectId: string;
  name: string;
  sourceType: NativeSourceType;
  byteSize: number;
  enabled: boolean;
  parseStatus: NativeParseStatus;
  parseMessage?: string | null;
  extractedCharCount: number;
  createdAt: string;
}

const kindFromNative: Record<NativeSourceType, SourceKind> = {
  txt: "TXT", pdf: "PDF", docx: "DOCX", pptx: "PPTX", image: "IMAGE", pasted_text: "TEXT",
};

async function fromNative(source: NativeSource): Promise<SourceDocument> {
  const extractedText = source.extractedCharCount > 0
    ? (await invokeNative<string>("read_source_text", { id: source.id })) ?? ""
    : "";
  const ready = source.parseStatus === "ready";
  return {
    id: source.id,
    name: source.name,
    kind: kindFromNative[source.sourceType],
    size: source.byteSize,
    enabled: source.enabled,
    status: ready ? "ready" : source.parseStatus === "failed" ? "error" : "unsupported",
    progress: ready ? 100 : 0,
    extractedText,
    statusMessage: source.parseMessage ?? (ready ? "解析完成" : source.parseStatus === "no_text" ? "未提取到文本" : "解析失败"),
    createdAt: source.createdAt,
  };
}

function activeProjectId(): string | undefined {
  return readLocal<string | undefined>("zhihua.activeProjectId", undefined);
}

export const sourceRepository = {
  async list(): Promise<SourceDocument[]> {
    if (isNativeRuntime()) {
      const projectId = activeProjectId();
      if (!projectId) return [];
      const sources = await invokeNative<NativeSource[]>("list_sources", { projectId });
      return Promise.all((sources ?? []).map(fromNative));
    }
    return readLocal(SOURCE_KEY, demoSources);
  },
  async listKnowledgePoints(): Promise<KnowledgePoint[]> {
    return (await invokeOptional<KnowledgePoint[]>("knowledge_point_list")) ?? readLocal(POINT_KEY, demoPoints);
  },
  async pickNative(): Promise<SourceDocument[] | undefined> {
    if (!isNativeRuntime()) return undefined;
    const projectId = activeProjectId();
    if (!projectId) throw new Error("请先在项目页创建或打开一个项目");
    const selected = await open({
      multiple: true,
      directory: false,
      filters: [{ name: "资料文件", extensions: ["pdf", "pptx", "docx", "txt", "png", "jpg", "jpeg", "webp", "bmp"] }],
    });
    if (!selected) return [];
    const paths = Array.isArray(selected) ? selected : [selected];
    const imported: SourceDocument[] = [];
    for (const filePath of paths) {
      const source = await invokeNative<NativeSource>("import_source_file", { input: { projectId, filePath } });
      if (source) imported.push(await fromNative(source));
    }
    return imported;
  },
  async fromFile(file: File): Promise<SourceDocument> {
    const kind = kindFromFile(file);
    const base: SourceDocument = {
      id: crypto.randomUUID(),
      name: file.name,
      kind: kind ?? "TEXT",
      size: file.size,
      enabled: true,
      status: "parsing",
      progress: 20,
      statusMessage: "等待解析",
      extractedText: "",
      createdAt: new Date().toISOString(),
    };
    if (!kind) return { ...base, status: "unsupported", progress: 0, statusMessage: "不支持此格式" };
    if (kind === "TXT") {
      return { ...base, status: "ready", progress: 100, statusMessage: "解析完成", extractedText: await file.text() };
    }
    if (kind === "IMAGE") {
      return { ...base, status: "ready", progress: 100, statusMessage: "图片已导入 · OCR 尚未接入", extractedText: "图片资料已保存。MVP 的图片 OCR 与版面识别尚未接入，当前可作为后续分镜的参考来源。" };
    }
    const native = await invokeOptional<SourceDocument>("source_import_and_parse", {
      file: { name: file.name, size: file.size, mimeType: file.type },
    });
    if (native) return native;
    return {
      ...base,
      status: "unsupported",
      progress: 0,
      statusMessage: `${kind} 解析器尚未接入`,
      extractedText: `文件已选择，但 ${kind} 文本与图片解析需要桌面端 source_import_and_parse command。当前未伪造提取结果。`,
    };
  },
  async fromText(text: string, name: string): Promise<SourceDocument> {
    const source: SourceDocument = { id: crypto.randomUUID(), name, kind: "TEXT", size: new Blob([text]).size, enabled: true, status: "ready", progress: 100, statusMessage: "文本已导入", extractedText: text, createdAt: new Date().toISOString() };
    if (isNativeRuntime()) {
      const projectId = activeProjectId();
      if (!projectId) throw new Error("请先在项目页创建或打开一个项目");
      const native = await invokeNative<NativeSource>("create_pasted_source", { input: { projectId, name, text } });
      if (native) return fromNative(native);
    }
    return source;
  },
  save(sources: SourceDocument[]): void { writeLocal(SOURCE_KEY, sources); },
  savePoints(points: KnowledgePoint[]): void { writeLocal(POINT_KEY, points); },
  async remove(id: string): Promise<void> { await invokeOptional("delete_source", { id }); },
  async setEnabled(id: string, enabled: boolean): Promise<void> { await invokeOptional("set_source_enabled", { input: { id, enabled } }); },
  async extractKnowledge(sourceIds: string[]): Promise<KnowledgePoint[] | undefined> {
    return invokeOptional<KnowledgePoint[]>("knowledge_extract", { sourceIds });
  },
};
