import {
  defaultProjectStyleProfile,
  normalizeProjectStyleProfile,
  type NewProjectInput,
  type ProjectStyleProfile,
  type ZhihuaProject,
} from "../domain/projects";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { SceneDraft } from "../domain/storyboard";
import { invokeNative, isNativeRuntime, readLocal, writeLocal } from "./nativeBridge";

const STORAGE_KEY = "zhihua.projects.v1";

const now = new Date().toISOString();
const demoProjects: ZhihuaProject[] = [
  ["为什么会打雷？", "通过生动的动画讲解雷电形成的原理。", 5, 42, "待导出", "lightning"],
  ["校园消防安全", "学习火灾的预防、逃生与自救知识。", 8, 75, "已完成", "school"],
  ["认识太阳系", "带领学生认识八大行星，探索宇宙的奥秘。", 12, 128, "生成中", "space"],
  ["预防流感", "了解流感的传播途径，学会科学防护。", 6, 63, "草稿", "health"],
  ["海洋生物的奇妙世界", "探索海洋生态系统，认识常见的海洋生物。", 10, 110, "编排中", "ocean"],
  ["植物是如何生长的", "从种子到大树，了解植物的生长过程。", 7, 88, "草稿", "plant"],
  ["气候变化与地球未来", "认识气候变化的影响，共同守护地球。", 9, 96, "已完成", "ice"],
  ["中国传统文化之美", "走进传统文化，感受中华文明的魅力。", 11, 132, "生成中", "culture"],
].map(([title, description, shotCount, durationSeconds, state, cover], index) => ({
  id: `demo-${index + 1}`,
  title: title as string,
  description: description as string,
  audience: index === 0 ? "小学高年级" : "",
  targetDurationSeconds: durationSeconds as number,
  shotCount: shotCount as number,
  durationSeconds: durationSeconds as number,
  state: state as ZhihuaProject["state"],
  styleProfile: defaultProjectStyleProfile(),
  cover: cover as string,
  createdAt: now,
  updatedAt: new Date(Date.now() - index * 3_600_000).toISOString(),
  lastOpenedAt: index === 0 ? now : undefined,
}));

function localList(): ZhihuaProject[] {
  return readLocal(STORAGE_KEY, demoProjects);
}

function persist(projects: ZhihuaProject[]): ZhihuaProject[] {
  writeLocal(STORAGE_KEY, projects);
  return projects;
}

type NativeProjectStatus = "draft" | "planning" | "generating" | "ready" | "completed";
interface NativeProject {
  id: string;
  title: string;
  audience?: string | null;
  targetDurationSec?: number | null;
  status: NativeProjectStatus;
  styleProfile?: Partial<ProjectStyleProfile> | null;
  projectDir: string;
  createdAt: string;
  updatedAt: string;
  lastOpenedAt?: string | null;
}

const stateFromNative: Record<NativeProjectStatus, ZhihuaProject["state"]> = {
  draft: "草稿",
  planning: "编排中",
  generating: "生成中",
  ready: "待导出",
  completed: "已完成",
};

interface NativeCandidateCover {
  localPath: string;
  selected: boolean;
  createdAt: string;
}

function projectState(project: NativeProject, scenes: SceneDraft[]): ZhihuaProject["state"] {
  if (project.status === "completed") return "已完成";
  if (scenes.some((scene) => scene.status === "generating" || Boolean(scene.pendingRequestId))) return "生成中";
  if (scenes.length && scenes.every((scene) => Boolean(scene.selectedVersionId))) return "待导出";
  if (scenes.length) return "编排中";
  return "草稿";
}

function fromNative(project: NativeProject, scenes: SceneDraft[] = [], coverUrl?: string): ZhihuaProject {
  const durationSeconds = Math.round(scenes.reduce((total, scene) => total + scene.targetDurationMs, 0) / 1000);
  return {
    id: project.id,
    title: project.title,
    description: project.audience ? `面向${project.audience}的科普视频项目。` : "本地科普视频项目。",
    audience: project.audience ?? "",
    targetDurationSeconds: project.targetDurationSec ?? null,
    shotCount: scenes.length,
    durationSeconds,
    state: scenes.length ? projectState(project, scenes) : stateFromNative[project.status],
    styleProfile: normalizeProjectStyleProfile(project.styleProfile),
    cover: "empty",
    coverUrl,
    projectDir: project.projectDir,
    createdAt: project.createdAt,
    updatedAt: project.updatedAt,
    lastOpenedAt: project.lastOpenedAt ?? undefined,
  };
}

export const projectRepository = {
  async list(): Promise<ZhihuaProject[]> {
    const native = await invokeNative<NativeProject[]>("list_projects");
    if (!native) return localList();
    return Promise.all(native.map(async (project) => {
      const scenes = (await invokeNative<SceneDraft[]>("list_storyboard_scenes", { projectId: project.id })) ?? [];
      let coverUrl: string | undefined;
      for (const scene of scenes) {
        const candidates = (await invokeNative<NativeCandidateCover[]>("list_candidate_versions", {
          input: { projectId: project.id, sceneId: scene.id },
        })) ?? [];
        const cover = candidates.find((item) => item.selected) ?? candidates[0];
        if (cover?.localPath) {
          coverUrl = convertFileSrc(cover.localPath);
          break;
        }
      }
      return fromNative(project, scenes, coverUrl);
    }));
  },

  async create(input: NewProjectInput): Promise<ZhihuaProject> {
    const native = await invokeNative<NativeProject>("create_project", {
      input: {
        title: input.title,
        audience: input.audience,
        targetDurationSec: input.targetDurationSeconds,
      },
    });
    if (native) return fromNative(native);
    const timestamp = new Date().toISOString();
    const project: ZhihuaProject = {
      id: crypto.randomUUID(),
      title: input.title.trim(),
      description: input.audience ? `面向${input.audience}的科普视频项目。` : "新建的科普视频项目。",
      audience: input.audience?.trim() ?? "",
      targetDurationSeconds: input.targetDurationSeconds ?? null,
      shotCount: 0,
      durationSeconds: 0,
      state: "草稿",
      styleProfile: defaultProjectStyleProfile(),
      cover: "lightning",
      createdAt: timestamp,
      updatedAt: timestamp,
      lastOpenedAt: timestamp,
    };
    persist([project, ...localList()]);
    return project;
  },

  async rename(id: string, title: string): Promise<void> {
    const native = await invokeNative<NativeProject>("rename_project", { id, title });
    if (native) return;
    persist(localList().map((item) => item.id === id ? { ...item, title, updatedAt: new Date().toISOString() } : item));
  },

  async duplicate(id: string): Promise<ZhihuaProject | undefined> {
    const native = await invokeNative<NativeProject>("duplicate_project", { id });
    if (native) return fromNative(native);
    const source = localList().find((item) => item.id === id);
    if (!source) return undefined;
    const timestamp = new Date().toISOString();
    const copy = { ...source, id: crypto.randomUUID(), title: `${source.title} - 副本`, state: "草稿" as const, createdAt: timestamp, updatedAt: timestamp, lastOpenedAt: timestamp };
    persist([copy, ...localList()]);
    return copy;
  },

  async remove(id: string): Promise<void> {
    if (isNativeRuntime()) {
      await invokeNative<void>("delete_project", { id });
      return;
    }
    persist(localList().filter((item) => item.id !== id));
  },

  async markOpened(id: string): Promise<void> {
    const native = await invokeNative<NativeProject>("open_project", { id });
    if (native) {
      writeLocal("zhihua.activeProjectId", id);
      return;
    }
    const timestamp = new Date().toISOString();
    persist(localList().map((item) => item.id === id ? { ...item, lastOpenedAt: timestamp, updatedAt: timestamp } : item));
    writeLocal("zhihua.activeProjectId", id);
  },

  async updateStyleProfile(id: string, styleProfile: ProjectStyleProfile): Promise<ZhihuaProject | undefined> {
    const normalized = normalizeProjectStyleProfile(styleProfile);
    const native = await invokeNative<NativeProject>("update_project", {
      input: { id, styleProfile: normalized },
    });
    if (native) return fromNative(native);
    const timestamp = new Date().toISOString();
    const projects = localList().map((item) => item.id === id
      ? { ...item, styleProfile: normalized, updatedAt: timestamp }
      : item);
    persist(projects);
    return projects.find((item) => item.id === id);
  },

  async active(): Promise<ZhihuaProject | undefined> {
    const id = readLocal<string | undefined>("zhihua.activeProjectId", undefined);
    if (!id) return undefined;
    return (await this.list()).find((project) => project.id === id);
  },
};
