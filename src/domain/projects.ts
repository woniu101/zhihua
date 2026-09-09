export type ProjectState = "草稿" | "编排中" | "生成中" | "待导出" | "已完成";

export interface ZhihuaProject {
  id: string;
  title: string;
  description: string;
  audience: string;
  targetDurationSeconds: number | null;
  shotCount: number;
  durationSeconds: number;
  state: ProjectState;
  cover: string;
  updatedAt: string;
  createdAt: string;
  lastOpenedAt?: string;
  projectDir?: string;
}

export interface NewProjectInput {
  title: string;
  audience?: string;
  targetDurationSeconds?: number | null;
}

export const projectStates: Array<"全部" | ProjectState> = [
  "全部",
  "草稿",
  "编排中",
  "生成中",
  "待导出",
  "已完成",
];
