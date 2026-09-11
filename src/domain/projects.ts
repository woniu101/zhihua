export type ProjectState = "草稿" | "编排中" | "生成中" | "待导出" | "已完成";

export interface ProjectStyleProfile {
  preset: string;
  palette: string;
  lineAndMaterial: string;
  lighting: string;
  backgroundComplexity: string;
  forbiddenElements: string;
  locked: boolean;
}

export const defaultProjectStyleProfile = (): ProjectStyleProfile => ({
  preset: "",
  palette: "",
  lineAndMaterial: "",
  lighting: "",
  backgroundComplexity: "",
  forbiddenElements: "",
  locked: false,
});

export function normalizeProjectStyleProfile(value?: Partial<ProjectStyleProfile> | null): ProjectStyleProfile {
  return { ...defaultProjectStyleProfile(), ...(value ?? {}) };
}

export function projectStylePrompt(profile?: ProjectStyleProfile): string {
  if (!profile) return "";
  return [
    profile.preset && `整体风格：${profile.preset}`,
    profile.palette && `主色板：${profile.palette}`,
    profile.lineAndMaterial && `线条与材质：${profile.lineAndMaterial}`,
    profile.lighting && `光线：${profile.lighting}`,
    profile.backgroundComplexity && `背景复杂度：${profile.backgroundComplexity}`,
    profile.forbiddenElements && `禁止元素：${profile.forbiddenElements}`,
  ].filter(Boolean).join("；");
}

export interface ZhihuaProject {
  id: string;
  title: string;
  description: string;
  audience: string;
  targetDurationSeconds: number | null;
  shotCount: number;
  durationSeconds: number;
  state: ProjectState;
  styleProfile: ProjectStyleProfile;
  cover: string;
  coverUrl?: string;
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
