export type SourceKind = "PDF" | "PPTX" | "DOCX" | "TXT" | "IMAGE" | "TEXT";
export type SourceParseStatus = "queued" | "parsing" | "ready" | "error" | "unsupported";

export interface SourceRef {
  sourceId: string;
  sourceName: string;
  location: string;
}

export interface SourceDocument {
  id: string;
  name: string;
  kind: SourceKind;
  size: number;
  pageCount?: number;
  enabled: boolean;
  status: SourceParseStatus;
  progress: number;
  extractedText: string;
  statusMessage: string;
  createdAt: string;
}

export interface KnowledgePoint {
  id: string;
  title: string;
  detail: string;
  sourceRefs: SourceRef[];
  needsConfirmation: boolean;
  confirmed: boolean;
}
