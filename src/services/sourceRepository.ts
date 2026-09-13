import type { KnowledgePoint, SourceDocument, SourceKind } from "../domain/sources";
import type { SceneDraft } from "../domain/storyboard";
import { open } from "@tauri-apps/plugin-dialog";
import { invokeNative, invokeOptional, isNativeRuntime, readLocal, writeLocal } from "./nativeBridge";

function sourceStorageKey(projectId: string): string {
  return `zhihua.sources.${projectId}.v2`;
}

function pointStorageKey(projectId: string): string {
  return `zhihua.knowledgePoints.${projectId}.v2`;
}

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
    const projectId = activeProjectId();
    return projectId ? readLocal(sourceStorageKey(projectId), []) : [];
  },
  async listKnowledgePoints(): Promise<KnowledgePoint[]> {
    if (isNativeRuntime()) {
      const projectId = activeProjectId();
      if (!projectId) return [];
      return (await invokeNative<KnowledgePoint[]>("knowledge_point_list", { projectId })) ?? [];
    }
    const projectId = activeProjectId();
    return projectId ? readLocal(pointStorageKey(projectId), []) : [];
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
  save(sources: SourceDocument[]): void {
    const projectId = activeProjectId();
    if (projectId) writeLocal(sourceStorageKey(projectId), sources);
  },
  async savePoints(points: KnowledgePoint[]): Promise<void> {
    if (isNativeRuntime()) {
      const projectId = activeProjectId();
      if (!projectId) throw new Error("请先在项目页创建或打开一个项目");
      await invokeNative<KnowledgePoint[]>("knowledge_points_replace", { projectId, points });
      return;
    }
    const projectId = activeProjectId();
    if (projectId) writeLocal(pointStorageKey(projectId), points);
  },
  async remove(id: string): Promise<void> { await invokeOptional("delete_source", { id }); },
  async setEnabled(id: string, enabled: boolean): Promise<void> { await invokeOptional("set_source_enabled", { input: { id, enabled } }); },
  async extractKnowledge(sourceIds: string[], targetAudience: string, targetDurationSec: number): Promise<KnowledgePoint[] | undefined> {
    if (!isNativeRuntime()) return undefined;
    const projectId = activeProjectId();
    if (!projectId) throw new Error("请先在项目页创建或打开一个项目");
    return invokeNative<KnowledgePoint[]>("knowledge_extract", {
      input: { projectId, sourceIds, targetAudience, targetDurationSec },
    });
  },
  async createStoryboard(targetAudience: string, targetDurationSec: number): Promise<SceneDraft[] | undefined> {
    if (!isNativeRuntime()) return undefined;
    const projectId = activeProjectId();
    if (!projectId) throw new Error("请先在项目页创建或打开一个项目");
    return invokeNative<SceneDraft[]>("storyboard_generate_from_knowledge", {
      input: { projectId, targetAudience, targetDurationSec },
    });
  },
};
